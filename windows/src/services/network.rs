/// Windows 版由 mihomo 自动选择物理网卡（auto-detect-interface），
/// 因此不需要像 macOS 版那样解析默认路由接口名。
pub fn default_interface(_for_tun: bool) -> Result<Option<String>, String> {
    Ok(None)
}
