fn main() {
    embed_resource::compile("shell.rc", embed_resource::NONE)
        .manifest_optional()
        .unwrap();
}
