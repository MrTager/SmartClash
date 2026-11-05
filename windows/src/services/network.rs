use crate::services::command;
use std::path::Path;

/// Windows 版由 mihomo 自动选择物理网卡（auto-detect-interface），
/// 因此不需要像 macOS 版那样解析默认路由接口名。
pub fn default_interface(_for_tun: bool) -> Result<Option<String>, String> {
    Ok(None)
}

/// 检查是否真的存在已接管流量的 TUN 网卡：
/// 必须同时满足「默认流量被拆成 0.0.0.0/1 与 128.0.0.0/1 路由」和
/// 「对应网卡处于 Up 状态且是 Wintun / Mihomo 适配器」。
/// 仅探测 7890 代理端口无法证明 TUN 已接管系统流量。
pub fn tun_ready() -> Result<String, String> {
    let script = r#"
$ErrorActionPreference = 'SilentlyContinue'
$routes = Get-NetRoute -AddressFamily IPv4 |
    Where-Object { $_.DestinationPrefix -in @('0.0.0.0/0', '0.0.0.0/1', '128.0.0.0/1') }
foreach ($route in $routes) {
    $adapter = Get-NetAdapter -InterfaceIndex $route.InterfaceIndex
    if ($adapter -and $adapter.Status -eq 'Up' -and
        ($adapter.InterfaceDescription -like '*Wintun*' -or $adapter.Name -eq 'Mihomo')) {
        $addresses = (Get-NetIPAddress -InterfaceIndex $adapter.InterfaceIndex -AddressFamily IPv4 |
            Select-Object -ExpandProperty IPAddress) -join ','
        Write-Output ("{0}|{1}|{2}" -f $adapter.Name, $adapter.InterfaceDescription, $addresses)
        exit 0
    }
}
Write-Output 'no-tun-route'
exit 1
"#;
    let output = command::run(
        Path::new("powershell.exe"),
        &[
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-Command".into(),
            script.into(),
        ],
    )
    .map_err(|e| format!("无法检查 TUN 网卡：{e}"))?;
    let detail = output.text.trim();
    if output.code == 0 && !detail.is_empty() {
        Ok(detail.to_string())
    } else {
        Err("未检测到已接管流量的 TUN 网卡或路由".into())
    }
}
