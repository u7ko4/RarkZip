fn main() {
    // 资源 ID 1 同时被 GPUI 的窗口类（LoadImageW(1)）和资源管理器使用。
    // 只有 Windows 目标需要嵌入 .rc 资源；Linux/macOS 上 embed-resource
    // 甚至不是构建依赖，因此用编译期 cfg 直接排除这段代码。
    #[cfg(windows)]
    embed_resource::compile("app.rc", embed_resource::NONE)
        .manifest_optional()
        .unwrap();
}
