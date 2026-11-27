# Provenance

Record where every package in this bucket came from and how you checked it.
Keep one row per package, and add notes when the evidence needs more space.
This bucket does not mirror or redistribute an archive unless you choose to
publish one yourself.

## Sources

- `github-release`: the release asset that the package source names.
- `https`: a URL that serves the exact archive.
- `nexus`: one Nexus Mods file, pinned by `mod_id` and `file_id`.
- `git`: a repository and a full commit.
- `manual`: a local archive that a user supplies with `--file`.

## Packages

| Package | Version | Source | Upstream | Archive | BLAKE3 pin | Checked |
| --- | --- | --- | --- | --- | --- | --- |
| `<game-id>/<package-id>` | `<version>` | `<source>` | `<upstream URL>` | `<asset>` | `<64 lowercase hex>` | `<YYYY-MM-DD and method>` |

The BLAKE3 pin is the value that `modn catalog hash <archive>` prints. It must
match the bytes that users download. Recompute it whenever the archive changes.

## Compatibility

Note the game versions that each package targets, the dependencies that its
manifest declares, and any package that it conflicts with. Say where the
upstream author documents each requirement.

## Checks

Describe how you inspected each archive: the disposable game directory, the
install and uninstall order, and any executable or native injector that the
package ships. A package that ships an executable needs a separate approval
note, because Modn asks the user before it installs one.
