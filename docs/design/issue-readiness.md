# Redesign issue readiness and artifact index

Audit snapshot of artifacts and missing evidence. Issue state and native blockers are not recorded here because they go stale; read them live as described in the Pickup step of the [implementation workflow](implementation-workflow.md#pickup). [Specification #68](https://github.com/naicoi92/iroh-tunnel/issues/68) is shared scope; issue bodies contain acceptance checklists. No proposal is automatically approved.

| Issue | Artifact | Missing evidence / disposition |
|---|---|---|
| [#68](https://github.com/naicoi92/iroh-tunnel/issues/68) Iroh Tunnel redesign specification | Issue body: scope and acceptance; shared #68 | Parent: redesign incomplete |
| [#69](https://github.com/naicoi92/iroh-tunnel/issues/69) UDP P2P transport — wayfinding map | Issue body: scope and acceptance; shared #68 | Map: UDP decisions remain open |
| [#70](https://github.com/naicoi92/iroh-tunnel/issues/70) UDP response mapping and endpoint behavior evidence | Issue body: scope and acceptance; shared #68 | Research complete; follow-on decisions remain open |
| [#71](https://github.com/naicoi92/iroh-tunnel/issues/71) UDP flow identity and target-visible behavior | [Decision packet](https://github.com/naicoi92/iroh-tunnel/issues/71#issuecomment-6490138952); issue AC | Flow lifecycle, filtering, reuse |
| [#72](https://github.com/naicoi92/iroh-tunnel/issues/72) Iroh datagram and stream transport evidence | Issue body: scope and acceptance; shared #68 | Research complete; follow-on decisions remain open |
| [#73](https://github.com/naicoi92/iroh-tunnel/issues/73) UDP payload size and tunnel transport policy | Issue body: scope and acceptance; shared #68 | Transport, size policy and benchmark |
| [#74](https://github.com/naicoi92/iroh-tunnel/issues/74) UDP resource limits and end-to-end verification | Issue body: scope and acceptance; shared #68 | Resource and verification policy |
| [#75](https://github.com/naicoi92/iroh-tunnel/issues/75) Decide tunnel wire compatibility contract | Issue body: scope and acceptance; shared #68 | Decision resolved; test 0.4 TCP compatibility in #85 |
| [#76](https://github.com/naicoi92/iroh-tunnel/issues/76) Decide TCP failure and shutdown lifecycle | [Decision packet](https://github.com/naicoi92/iroh-tunnel/issues/76#issuecomment-6490139738); issue AC | Decision closed; numeric defaults and E2E handed to #86 |
| [#77](https://github.com/naicoi92/iroh-tunnel/issues/77) Decide role ownership and stale status policy | [Decision packet](https://github.com/naicoi92/iroh-tunnel/issues/77#issuecomment-6490140456); issue AC | Detection mechanism (#87) |
| [#78](https://github.com/naicoi92/iroh-tunnel/issues/78) Integrate UDP transport and lifecycle decisions | Issue body: scope and acceptance; shared #68 | Integrate UDP decisions and benchmark |
| [#79](https://github.com/naicoi92/iroh-tunnel/issues/79) Decide service trust and Windows user background | [Decision packet](https://github.com/naicoi92/iroh-tunnel/issues/79#issuecomment-6490141217); issue AC | Trust model and Windows background |
| [#80](https://github.com/naicoi92/iroh-tunnel/issues/80) Decide update recovery and signing trust | [Decision packet](https://github.com/naicoi92/iroh-tunnel/issues/80#issuecomment-6490141958); issue AC | Replacement, rollback and trust rotation |
| [#81](https://github.com/naicoi92/iroh-tunnel/issues/81) Run a TCP tunnel with quick flags | Issue body: scope and acceptance; shared #68 | Implementation and end-to-end evidence not yet verified |
| [#82](https://github.com/naicoi92/iroh-tunnel/issues/82) Run independently packaged role binaries | Issue body: scope and acceptance; shared #68 | Implementation and end-to-end evidence not yet verified |
| [#83](https://github.com/naicoi92/iroh-tunnel/issues/83) Run configured roles with legacy multiplex compatibility | Issue body: scope and acceptance; shared #68 | Implementation and end-to-end evidence not yet verified |
| [#84](https://github.com/naicoi92/iroh-tunnel/issues/84) Resolve configuration paths and migrate macOS identity safely | Issue body: scope and acceptance; shared #68 | Implementation and end-to-end evidence not yet verified |
| [#85](https://github.com/naicoi92/iroh-tunnel/issues/85) Verify TCP interoperability with iroh-tunnel 0.4 | Issue body: scope and acceptance; shared #68 | Implementation and end-to-end evidence not yet verified |
| [#86](https://github.com/naicoi92/iroh-tunnel/issues/86) Multiplex TCP clients with agreed failure behavior | Issue body: scope and acceptance; shared #68 | Implementation and end-to-end evidence not yet verified |
| [#87](https://github.com/naicoi92/iroh-tunnel/issues/87) Enforce role ownership and report status | Issue body: scope and acceptance; shared #68 | Implementation and end-to-end evidence not yet verified |
| [#88](https://github.com/naicoi92/iroh-tunnel/issues/88) Forward UDP clients through a P2P tunnel | Issue body: scope and acceptance; shared #68 | Implementation and end-to-end evidence not yet verified |
| [#89](https://github.com/naicoi92/iroh-tunnel/issues/89) Bound UDP flow lifecycle and resource use | Issue body: scope and acceptance; shared #68 | Implementation and end-to-end evidence not yet verified |
| [#90](https://github.com/naicoi92/iroh-tunnel/issues/90) Install and manage a user service | Issue body: scope and acceptance; shared #68 | Implementation and end-to-end evidence not yet verified |
| [#91](https://github.com/naicoi92/iroh-tunnel/issues/91) Install system service across privilege boundary | Issue body: scope and acceptance; shared #68 | Implementation and end-to-end evidence not yet verified |
| [#92](https://github.com/naicoi92/iroh-tunnel/issues/92) Run Windows roles in user background | Issue body: scope and acceptance; shared #68 | Implementation and end-to-end evidence not yet verified |
| [#93](https://github.com/naicoi92/iroh-tunnel/issues/93) Install binaries using shell and PowerShell | Issue body: scope and acceptance; shared #68 | Implementation and end-to-end evidence not yet verified |
| [#94](https://github.com/naicoi92/iroh-tunnel/issues/94) Publish separate role packages and images | Issue body: scope and acceptance; shared #68 | Implementation and end-to-end evidence not yet verified |
| [#95](https://github.com/naicoi92/iroh-tunnel/issues/95) Verify and replace one standalone executable | Issue body: scope and acceptance; shared #68 | Implementation and end-to-end evidence not yet verified |
| [#96](https://github.com/naicoi92/iroh-tunnel/issues/96) Respect package and container update ownership | Issue body: scope and acceptance; shared #68 | Implementation and end-to-end evidence not yet verified |
| [#97](https://github.com/naicoi92/iroh-tunnel/issues/97) Verify cross-platform redesign release end to end | Issue body: scope and acceptance; shared #68 | Implementation and end-to-end evidence not yet verified |

Benchmarks and full E2E have not been performed. Local design documents are not necessarily published.
