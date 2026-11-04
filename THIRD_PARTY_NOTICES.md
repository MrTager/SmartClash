# Third-party notices

SmartClash source: GPL-3.0-or-later, 2026 SmartClash contributors. The full GPL v3 is in LICENSE.

## Mihomo

Project: https://github.com/MetaCubeX/mihomo
Pinned version: v1.19.33
License: GNU General Public License v3.0 (GPL-3.0)
Corresponding source: https://github.com/MetaCubeX/mihomo/tree/v1.19.33
Source archive: https://github.com/MetaCubeX/mihomo/archive/refs/tags/v1.19.33.tar.gz
The build fetches an unmodified official Intel macOS executable and verifies its release asset digest. It is not included in the source archive. When redistributing the built application, also provide corresponding source and required license notices under the applicable GPL terms.

## Yams

Project: https://github.com/jpsim/Yams
Version: 6.2.2
License: MIT
Source/license: https://github.com/jpsim/Yams/tree/6.2.2
Includes libyaml; upstream Yams source contains its notices. A copy of the upstream license is obtained by the package manager in the resolved dependency. For redistribution, include the upstream notices in addition to this attribution. The packaging script copies the resolved notices into the application.

## Natural Earth

The bundled EarthLand.geojson contains Natural Earth 1:110m land polygons, public domain.
Source: https://github.com/nvkelso/natural-earth-vector/blob/master/geojson/ne_110m_land.geojson
Terms: https://www.naturalearthdata.com/about/terms-of-use/
The application draws its globe texture locally from these polygons; no remote map tiles are required.

## IP location lookup

Approximate public-IP locations are obtained from https://ipwho.is/ and cached locally for 24 hours.
Only public IP addresses are sent to the provider, without subscription credentials, node names, or requested hostnames.

## Shadowrocket routing rules

Project and corresponding source: https://github.com/GMOogway/shadowrocket-rules
License: GNU General Public License v3.0 (GPL-3.0); see the upstream LICENSE.
The application downloads the upstream DIRECT, PROXY and REJECT modules at runtime and converts supported rules locally. It checks for updates hourly while running. Rule snapshots are not bundled in the application or committed in this repository. Upstream URL-REGEX, USER-AGENT and malformed entries are not used; the application reports the skipped count. jsDelivr is an upstream-listed fallback mirror.
