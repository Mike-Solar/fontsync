fn main() {
    #[cfg(target_os = "windows")]
    {
        // Windows 资源文件包含清单，确保安装字体时使用管理员权限
        embed_resource::compile("packaging/windows/fontsync.rc", std::iter::empty::<&str>());
    }
}
