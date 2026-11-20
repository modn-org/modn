# Staged official themes

This directory stages official game themes for the `main` bucket, outside the
`buckets/main` submodule. Each theme folder is a drop-in copy of
`themes/<game-id>/<theme-id>/` in `modn-org/main`. Today it holds one theme:
Cyberpunk 2077's `night-city`, which changes layout, not only colors.

Copy `cyberpunk-2077/night-city/` to `themes/cyberpunk-2077/night-city/` of
`modn-org/main`. `modn theme check <dir>` must pass there; the CLI commands
arrive with the CLI milestone work. The `modn-core` test
`official_theme_layout` runs the real theme API v1 rule engine on this folder.

## Font and art

`assets/chakra-petch-semibold.woff2` is Chakra Petch SemiBold from Google Fonts
(`ofl/chakrapetch`), under the SIL Open Font License 1.1. `LICENSE` holds the
OFL text and `NOTICE.md` names the copyright. The icon (128x128) and the grid
tile (32x32) are original generated art. From the repository root:

```text
curl -O https://raw.githubusercontent.com/google/fonts/main/ofl/chakrapetch/ChakraPetch-SemiBold.ttf
curl -O https://raw.githubusercontent.com/google/fonts/main/ofl/chakrapetch/OFL.txt
uv run --no-project --with fonttools --with brotli python -m fontTools.ttLib.woff2 compress ChakraPetch-SemiBold.ttf -o examples/themes/cyberpunk-2077/night-city/assets/chakra-petch-semibold.woff2
python examples/themes/tools/make_assets.py examples/themes/cyberpunk-2077/night-city/assets
```
