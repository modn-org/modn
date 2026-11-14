# Verify fixtures

`cyberpunk-2077/` is a static copy of the first-party Cyberpunk 2077 game
module. The verifiers copy it into a disposable contract v2 bucket and never
read the repository's `buckets/main` submodule at run time. The copy is test
input only; Modn embeds no game module.
