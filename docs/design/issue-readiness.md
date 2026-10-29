# Redesign issue readiness and artifact index

Audit snapshot; live issue status and native blockers are authoritative. [Specification #68](https://github.com/naicoi92/iroh-tunnel/issues/68) is shared scope; issue bodies contain acceptance checklists. No proposal is automatically approved.

| Issue | State / open blockers | Artifact | Missing evidence / disposition |
|---|---|---|---|
| [#68](https://github.com/naicoi92/iroh-tunnel/issues/68) Iroh Tunnel redesign specification | open; none | Issue body: scope and acceptance; shared #68 | Parent: redesign incomplete |
| [#69](https://github.com/naicoi92/iroh-tunnel/issues/69) UDP P2P transport — wayfinding map | open; none | Issue body: scope and acceptance; shared #68 | Map: UDP decisions remain open |
| [#70](https://github.com/naicoi92/iroh-tunnel/issues/70) UDP response mapping and endpoint behavior evidence | closed; none | Issue body: scope and acceptance; shared #68 | Research complete; follow-on decisions remain open |
| [#71](https://github.com/naicoi92/iroh-tunnel/issues/71) UDP flow identity and target-visible behavior | open; none | [Decision packet](https://github.com/naicoi92/iroh-tunnel/issues/71#issuecomment-6490138952); issue AC | Flow lifecycle, filtering, reuse |
| [#72](https://github.com/naicoi92/iroh-tunnel/issues/72) Iroh datagram and stream transport evidence | closed; none | Issue body: scope and acceptance; shared #68 | Research complete; follow-on decisions remain open |
| [#73](https://github.com/naicoi92/iroh-tunnel/issues/73) UDP payload size and tunnel transport policy | open; none | Issue body: scope and acceptance; shared #68 | Transport, size policy and benchmark |
| [#74](https://github.com/naicoi92/iroh-tunnel/issues/74) UDP resource limits and end-to-end verification | open; #71, #73 | Issue body: scope and acceptance; shared #68 | Resource and verification policy |
| [#75](https://github.com/naicoi92/iroh-tunnel/issues/75) Decide tunnel wire compatibility contract | closed; none | Issue body: scope and acceptance; shared #68 | Decision resolved; test 0.4 TCP compatibility in #85 |
| [#76](https://github.com/naicoi92/iroh-tunnel/issues/76) Decide TCP failure and shutdown lifecycle | closed; none | [Decision packet](https://github.com/naicoi92/iroh-tunnel/issues/76#issuecomment-6490139738); issue AC | Decision closed; numeric defaults and E2E handed to #86 |
| [#77](https://github.com/naicoi92/iroh-tunnel/issues/77) Decide role ownership and stale status policy | open; none | [Decision packet](https://github.com/naicoi92/iroh-tunnel/issues/77#issuecomment-6490140456); issue AC | Cross-user lock feasibility and stale state |
| [#78](https://github.com/naicoi92/iroh-tunnel/issues/78) Integrate UDP transport and lifecycle decisions | open; #71, #73, #74 | Issue body: scope and acceptance; shared #68 | Integrate UDP decisions and benchmark |
| [#79](https://github.com/naicoi92/iroh-tunnel/issues/79) Decide service trust and Windows user background | open; none | [Decision packet](https://github.com/naicoi92/iroh-tunnel/issues/79#issuecomment-6490141217); issue AC | Runtime account, trust model and Windows background |
| [#80](https://github.com/naicoi92/iroh-tunnel/issues/80) Decide update recovery and signing trust | open; none | [Decision packet](https://github.com/naicoi92/iroh-tunnel/issues/80#issuecomment-6490141958); issue AC | Replacement, rollback and trust rotation |
| [#81](https://github.com/naicoi92/iroh-tunnel/issues/81) Run a TCP tunnel with quick flags | open; none | Issue body: scope and acceptance; shared #68 | Implementation and end-to-end evidence not yet verified |
| [#82](https://github.com/naicoi92/iroh-tunnel/issues/82) Run independently packaged role binaries | open; #81 | Issue body: scope and acceptance; shared #68 | Implementation and end-to-end evidence not yet verified |
| [#83](https://github.com/naicoi92/iroh-tunnel/issues/83) Run configured roles with legacy multiplex compatibility | open; #81 | Issue body: scope and acceptance; shared #68 | Implementation and end-to-end evidence not yet verified |
| [#84](https://github.com/naicoi92/iroh-tunnel/issues/84) Resolve configuration paths and migrate macOS identity safely | open; #83 | Issue body: scope and acceptance; shared #68 | Implementation and end-to-end evidence not yet verified |
| [#85](https://github.com/naicoi92/iroh-tunnel/issues/85) Verify TCP interoperability with iroh-tunnel 0.4 | open; #81 | Issue body: scope and acceptance; shared #68 | Implementation and end-to-end evidence not yet verified |
| [#86](https://github.com/naicoi92/iroh-tunnel/issues/86) Multiplex TCP clients with agreed failure behavior | open; #81 | Issue body: scope and acceptance; shared #68 | Implementation and end-to-end evidence not yet verified |
| [#87](https://github.com/naicoi92/iroh-tunnel/issues/87) Enforce role ownership and report status | open; #77, #82 | Issue body: scope and acceptance; shared #68 | Implementation and end-to-end evidence not yet verified |
| [#88](https://github.com/naicoi92/iroh-tunnel/issues/88) Forward UDP clients through a P2P tunnel | open; #78, #81 | Issue body: scope and acceptance; shared #68 | Implementation and end-to-end evidence not yet verified |
| [#89](https://github.com/naicoi92/iroh-tunnel/issues/89) Bound UDP flow lifecycle and resource use | open; #78, #88 | Issue body: scope and acceptance; shared #68 | Implementation and end-to-end evidence not yet verified |
| [#90](https://github.com/naicoi92/iroh-tunnel/issues/90) Install and manage a user service | open; #84, #87 | Issue body: scope and acceptance; shared #68 | Implementation and end-to-end evidence not yet verified |
| [#91](https://github.com/naicoi92/iroh-tunnel/issues/91) Install system service across privilege boundary | open; #79, #84, #87 | Issue body: scope and acceptance; shared #68 | Implementation and end-to-end evidence not yet verified |
| [#92](https://github.com/naicoi92/iroh-tunnel/issues/92) Run Windows roles in user background | open; #79, #84, #87 | Issue body: scope and acceptance; shared #68 | Implementation and end-to-end evidence not yet verified |
| [#93](https://github.com/naicoi92/iroh-tunnel/issues/93) Install binaries using shell and PowerShell | open; #82 | Issue body: scope and acceptance; shared #68 | Implementation and end-to-end evidence not yet verified |
| [#94](https://github.com/naicoi92/iroh-tunnel/issues/94) Publish separate role packages and images | open; #82 | Issue body: scope and acceptance; shared #68 | Implementation and end-to-end evidence not yet verified |
| [#95](https://github.com/naicoi92/iroh-tunnel/issues/95) Verify and replace one standalone executable | open; #80, #82 | Issue body: scope and acceptance; shared #68 | Implementation and end-to-end evidence not yet verified |
| [#96](https://github.com/naicoi92/iroh-tunnel/issues/96) Respect package and container update ownership | open; #94 | Issue body: scope and acceptance; shared #68 | Implementation and end-to-end evidence not yet verified |
| [#97](https://github.com/naicoi92/iroh-tunnel/issues/97) Verify cross-platform redesign release end to end | open; #84, #85, #86, #87, #88, #89, #90, #91, #92, #93, #94, #95, #96 | Issue body: scope and acceptance; shared #68 | Implementation and end-to-end evidence not yet verified |

Research #70 and #72 and decisions #75 and #76 are closed; implementation issues remain open. Benchmarks and full E2E have not been performed. Local design documents are not necessarily published.
