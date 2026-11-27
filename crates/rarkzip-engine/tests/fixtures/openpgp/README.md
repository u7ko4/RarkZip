# OpenPGP test fixtures

These keys and signatures are generated solely for the test suite.

- `public.asc`, `data.txt.asc`, `data.txt.sig`, `data.asc` — verification samples.
- `signer-secret.asc` — passphrase-protected test signing key; passphrase: `test-passphrase`.
- `signer-secret-unencrypted.asc` — unencrypted test key used to verify that unencrypted
  private keys are rejected on import.
- `signer-public.asc` — public counterpart of the signing key.

Do not use any of these keys for real data.
