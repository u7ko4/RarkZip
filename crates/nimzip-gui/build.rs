fn main() {
    // 资源 ID 1 同时被 GPUI 的窗口类（LoadImageW(1)）和资源管理器使用。
    embed_resource::compile("app.rc", embed_resource::NONE)
        .manifest_optional()
        .unwrap();
}
