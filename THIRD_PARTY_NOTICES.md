# Third-party notices

SmartClash source: GPL-3.0-or-later, 2026 SmartClash contributors. The full GPL v3 is in LICENSE.

## Clash Verge Rev desktop foundation

Project: https://github.com/clash-verge-rev/clash-verge-rev
Baseline: v2.5.10, commit c83f7ad529c7c9cb3d2d54a0955ef8c1b22a411b.
License: GPL-3.0-only. The vendored desktop source and original license are in
`verge/`, with baseline metadata in `verge/UPSTREAM.json`. SmartClash modifications
include intelligent routing, subscription aggregation and a Natural Earth globe.
The upstream notices, dependency manifests and locked dependency versions remain
available in that directory. Distribute the corresponding modified source and
these notices with binaries as required by the GPL.

## Mihomo

Project and corresponding source: https://github.com/MetaCubeX/mihomo
License: GNU General Public License v3.0 (GPL-3.0).
The desktop resource preparation script downloads official Mihomo releases.
Release versions are resolved by `verge/scripts/prebuild.mjs`; distributions
must identify the bundled core version and provide its corresponding source
and license notices. Local core binaries are not committed to this repository.

## Clash Verge service

Project and corresponding source: https://github.com/clash-verge-rev/clash-verge-service-ipc
Pinned protocol/release: v2.7.7, as declared in `verge/src-tauri/Cargo.toml`.
Preserve the upstream service license and distribution notices with its binaries.

## Natural Earth

The bundled `verge/src/components/smartclash/world-land.json` contains simplified
Natural Earth 1:110m land outlines, public domain.
Source: https://github.com/nvkelso/natural-earth-vector/blob/master/geojson/ne_110m_land.geojson
Terms: https://www.naturalearthdata.com/about/terms-of-use/
The application renders these outlines locally without remote map tiles.

## IP location lookup

Approximate public-IP locations are obtained from https://ipwho.is/ and cached
in memory for 24 hours. Only public IP addresses are sent to the provider,
without subscription credentials, node names or requested hostnames.

## SmartClash routing rules

Project and corresponding source: https://github.com/MrTager/SmartClash-Rules
License: GNU General Public License v3.0 (GPL-3.0); see that repository for upstream
data attribution. Optional Mihomo rule providers download `rules/direct.yaml`,
`rules/custom-direct.yaml` and `rules/cncidr.yaml`, refreshing hourly while the
core runs. Domestic domains and IPs route directly; unmatched traffic uses
`SMARTCLASH`. Rule snapshots are not committed to this repository.
