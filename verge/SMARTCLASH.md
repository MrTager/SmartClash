# SmartClash desktop

This desktop implementation is based directly on Clash Verge Rev v2.5.10, commit
`c83f7ad529c7c9cb3d2d54a0955ef8c1b22a411b`. Upstream source and GPL notices remain
in this directory. `UPSTREAM.json` records the baseline for future updates.

The Tauri/React desktop implementation in this directory is the sole client
implementation in the repository. Its application data directory is
`io.github.mrtager.smartclash` (`.dev` for development), separate from Clash Verge
Rev and the legacy SmartClash data. Existing subscriptions are not migrated
automatically; import them on the Profiles page.

## Features

- Upstream profile management, core lifecycle, system proxy, TUN, connections,
  traffic, logs and rule management remain available.
- SmartClash adds a `SMARTCLASH` selector to subscription selectors. Select it
  in a business selector on the Proxies page, or enable SmartClash rules, to send
  business traffic through the chosen smart route. Enabling the feature alone
  preserves the subscription's existing business selections.
- Low latency uses a real HTTPS 204 probe. High speed downloads at most six
  1 MiB samples per explicit optimization. AI checks the real OpenAI models
  endpoint without an API key; structured authentication errors establish
  reachability, not account or website access. Unknown challenges and 429
  responses are not treated as definitive endpoint failures.
- Background monitoring preserves a healthy route, warms up to two standby
  candidates, uses a 15 second freshness window, and backs off failed candidates
  for 60 seconds. Ordinary failure needs two checks; explicit AI failure needs
  one. Automatic switches have a 30 second cooldown. Each round has a 45 second
  budget, with up to three isolated probes at once. High speed background checks
  use HTTPS only; bandwidth downloads require explicit optimization.
- Manual selection disables automatic selection. Explicit optimization still
  works with automatic selection disabled. Runtime generation and source
  configuration are checked before applying a measured candidate.
- Select up to 64 additional cached profiles to aggregate inline nodes. Node
  identities exclude display names; name collisions get fingerprint suffixes.
  Additional profile updates rebuild the active aggregate. Provider-only sources
  do not support isolated optimization or local aggregate export. Cross-profile
  references to source strategy groups must be replaced with explicit nodes.
- Explicit entry/exit combinations support up to 128 TCP chains. Optional
  automatic exploration adds up to 48 bounded two-hop candidates. It does not
  reproduce the legacy unbounded-depth exploration strategy.
- Optional SmartClash rules replace subscription routing with local/private
  direct, domestic domain/IP direct and `MATCH,SMARTCLASH`. The three providers
  come from `MrTager/SmartClash-Rules` and refresh hourly while Mihomo runs.
- Local subscription export runs on the existing loopback server and requires
  a private random token. It exports nodes, the smart selector and routing rules,
  excluding local TUN, listener and controller credentials. Exports contain proxy
  credentials. The local URL works only while the application runs on this PC.
- The globe uses bundled Natural Earth land outlines, real node locations and
  each connection's own chain and destination IP. Drag/arrow keys rotate, wheel
  zooms, double click/Home resets, and request chips focus arcs. Cyan marks nodes;
  orange marks destinations. Unknown/private/fake-IP destinations remain
  unmapped; no geographic guesses are generated from node names.
- Location lookup sends only resolved public IPs to ipwho.is. Successful
  locations remain in memory for 24 hours, failures back off five minutes.
  TUN fake-IP connections can remain unmapped. Positions are approximate and
  arcs are not physical traceroutes. Reduced-motion preferences disable motion.

## Build

Use the upstream Rust 1.99.1 toolchain, Node and pnpm versions from the project
manifests. On Windows the normal upstream target is MSVC and requires the C++
build tools.

```powershell
cd verge
pnpm install --frozen-lockfile
pnpm prebuild x86_64-pc-windows-msvc
pnpm web:build
pnpm build
```

For an unpacked Windows preview, run `./build-preview.ps1` after `prebuild`. It
copies the executable, WebView loader, both cores, service/Geo resources and
license notices into the repository's `dist/SmartClash-Verge-Preview`. This is
separate from Vite's `verge/dist`, which is emptied during frontend builds. The
local preview is a development build, not a signed installer.

`prebuild` downloads the matching upstream service, Mihomo sidecars and Geo
assets; network access to GitHub release assets is required. Do not use an older
Clash Verge service as a replacement for the pinned service protocol. Installing
or replacing the shared system service is an explicit application action and may
affect another running Clash Verge instance. The service prompt can be deferred
to browse the interface without making system changes.

For the existing GNU toolchain on this workstation, code validation uses:

```powershell
cargo +stable-x86_64-pc-windows-gnu test -p clash-verge --lib --features clippy --locked
cargo +stable-x86_64-pc-windows-gnu run -p clash-verge --example frontend-contract --features clippy --locked
pnpm exec biome format --write src/services/bindings.ts
pnpm typecheck
pnpm test
pnpm arch:check
```

The `clippy` feature skips Tauri resource preparation for code/tests, as upstream
already does; it does not validate installer resources. The GNU test manifest
fix in `build.rs` enables Windows common controls for test executables.

## Current validation and limitations

Frontend production build, frontend tests, Rust library tests and architecture
checks have run locally. The Windows application has been compiled and launched,
and the globe has rendered in the real desktop window. Drag rotation and wheel
zoom were verified in the unpacked preview. No existing system
service, proxy settings or TUN configuration were replaced for validation.

The installed Clash Verge service on this PC is incompatible with the pinned
v2.5.10 service protocol and already owns an active session. Consequently, real
proxy/TUN takeover and live-node failover have not been accepted on this build.
The application correctly reports that conflict. macOS/Linux packaging and
service integration still require platform testing.

Signed SmartClash application updates are disabled until a separate signed
release feed exists. Upstream application updates would overwrite SmartClash.
Mihomo core update controls retain upstream behavior.
