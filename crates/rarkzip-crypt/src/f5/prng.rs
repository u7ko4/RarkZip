use blake2::digest::Digest;
use blake2::Blake2b512;

const DIGEST_SIZE: usize = 64;
const CYCLE_COUNT: u64 = 10;

/// BLAKE2b-based digest random generator used by the F5 permutation.
struct DigestRandomGenerator {
    state: [u8; DIGEST_SIZE],
    seed: [u8; DIGEST_SIZE],
    state_counter: u64,
    seed_counter: u64,
}

impl DigestRandomGenerator {
    fn new() -> Self {
        Self {
            state: [0u8; DIGEST_SIZE],
            seed: [0u8; DIGEST_SIZE],
            state_counter: 1,
            seed_counter: 1,
        }
    }

    fn hash(parts: &[&[u8]]) -> [u8; DIGEST_SIZE] {
        let mut hasher = Blake2b512::new();
        for part in parts {
            hasher.update(part);
        }
        let output = hasher.finalize();
        let mut result = [0u8; DIGEST_SIZE];
        result.copy_from_slice(&output);
        result
    }

    fn add_seed_material(&mut self, seed: &[u8]) {
        self.seed = Self::hash(&[seed, &self.seed]);
    }

    fn generate_state(&mut self) {
        let counter = self.state_counter.to_le_bytes();
        self.state_counter = self.state_counter.wrapping_add(1);
        self.state = Self::hash(&[&counter, &self.state, &self.seed]);
        if self.state_counter % CYCLE_COUNT == 0 {
            let seed_counter = self.seed_counter.to_le_bytes();
            self.seed_counter = self.seed_counter.wrapping_add(1);
            self.seed = Self::hash(&[&self.seed, &seed_counter]);
        }
    }

    fn next_bytes(&mut self, output: &mut [u8]) {
        let mut state_offset = 0usize;
        self.generate_state();
        for byte in output.iter_mut() {
            if state_offset == self.state.len() {
                self.generate_state();
                state_offset = 0;
            }
            *byte = self.state[state_offset];
            state_offset += 1;
        }
    }
}

/// F5's buffered random source.
pub struct F5Random {
    generator: DigestRandomGenerator,
    buffer: [u8; 512],
    buffer_counter: usize,
}

impl F5Random {
    pub fn new(key: &[u8]) -> Self {
        let mut generator = DigestRandomGenerator::new();
        generator.add_seed_material(key);
        Self {
            generator,
            buffer: [0u8; 512],
            buffer_counter: 512,
        }
    }

    pub fn next_byte(&mut self) -> u8 {
        if self.buffer_counter >= 511 {
            self.generator.next_bytes(&mut self.buffer);
            self.buffer_counter = 0;
        } else {
            self.buffer_counter += 1;
        }
        self.buffer[self.buffer_counter]
    }

    pub fn next_value(&mut self, max_value: u32) -> u32 {
        debug_assert!(max_value > 0);
        let a = self.next_byte() as i8 as i32;
        let b = self.next_byte() as i8 as i32;
        let c = self.next_byte() as i8 as i32;
        let d = self.next_byte() as i8 as i32;
        let mut value = a | (b << 8) | (c << 16) | (d << 24);
        value %= max_value as i32;
        if value < 0 {
            value += max_value as i32;
        }
        value as u32
    }
}

/// Fisher-Yates permutation over the coefficient array.
pub struct Permutation {
    shuffled: Vec<usize>,
}

impl Permutation {
    pub fn new(size: usize, random: &mut F5Random) -> Self {
        let mut shuffled: Vec<usize> = (0..size).collect();
        let mut max_random = size;
        for _ in 0..size {
            let random_index = random.next_value(max_random as u32) as usize;
            max_random -= 1;
            shuffled.swap(random_index, max_random);
        }
        Self { shuffled }
    }

    pub fn get(&self, index: usize) -> usize {
        self.shuffled[index]
    }

    pub fn len(&self) -> usize {
        self.shuffled.len()
    }

    pub fn is_empty(&self) -> bool {
        self.shuffled.is_empty()
    }
}
