# Third-party notices

Modn Desktop uses GPUI Kit 0.9.0 by Longbridge and its contributors.
GPUI Kit, GPUI Base, GPUI Component, and bundled assets use Apache-2.0.
The matching GPUI pre-release family is 0.3.11 and uses Apache-2.0.
See licenses/LICENSE-APACHE for the license terms.

The bundled Lucide icons use ISC. See licenses/LICENSE-LUCIDE.
The app loads the platform fonts Segoe UI and Consolas. It does not redistribute fonts.

The root workspace Cargo.lock records transitive dependencies.
The Windows-only manifest pins GPUI Kit 0.9.0 and GPUI pre 0.3.11. Their license terms remain applicable.
A distributor must audit the locked dependency graph and retain required notices.
This source notices file does not certify a complete binary distribution license audit.

Sources:
- https://github.com/longbridge/gpui-kit/tree/v0.9.0
- https://github.com/longbridge/gpui-kit/blob/v0.9.0/LICENSE-APACHE
- https://github.com/longbridge/gpui-kit/blob/v0.9.0/crates/assets/LICENSE-LUCIDE
