"""Tests for rehash_main.py conversion and checks.

Run with `python .github/scripts/test_rehash_main.py`. That entry also
runs the safety tests in test_rehash_main_safety.py and
test_rehash_main_safety_failures.py.
"""

from rehash_test_support import *  # noqa: F401,F403


class ConversionTests(RehashTestCase):
    @NEEDS_MODN
    def test_valid_v1_bucket_converts_to_the_expected_v2_tree(self) -> None:
        before = snapshot(self.fixture.bucket)
        code, stdout, stderr = self.run_main(*self.convert_arguments())
        self.assertEqual(code, 0, stderr)
        self.assertIn("Rehashed 5 packages", stdout)

        files = {
            path.relative_to(self.out).as_posix()
            for path in self.out.rglob("*")
            if path.is_file()
        }
        self.assertEqual(
            files,
            {
                "bucket.json",
                "PROVENANCE.md",
                "games/fixture-game/module.json",
                "games/fixture-game/module.lua",
                "mods/fixture-game/git-mod.json",
                "mods/fixture-game/https-mod.json",
                "mods/fixture-game/manual-mod.json",
                "mods/fixture-game/manual-mod@0.9.json",
                "mods/fixture-game/release-mod.json",
            },
        )
        metadata = json.loads((self.out / "bucket.json").read_text(encoding="utf-8"))
        self.assertEqual(
            metadata, {"schema_version": 2, "name": "fixture", "title": "Fixture Main"}
        )
        for module in ("module.json", "module.lua"):
            self.assertEqual(
                (self.out / "games" / GAME / module).read_bytes(),
                (self.fixture.bucket / "games" / GAME / module).read_bytes(),
            )

        for name in ("https-mod", "release-mod", "manual-mod", "manual-mod@0.9"):
            expected = self.fixture.expected[name]
            manifest = json.loads(
                (self.out / "mods" / GAME / f"{name}.json").read_text(encoding="utf-8")
            )
            pin = self.modn_hash(self.fixture.archives_by_id[name])
            self.assertEqual(
                manifest,
                {
                    "schema_version": 2,
                    "id": expected["id"],
                    "game": GAME,
                    "name": expected["name"],
                    "version": expected["version"],
                    "archive_format": "zip",
                    "blake3": pin,
                    "sources": [expected["source"]],
                    "redistributable": False,
                    "upstream_sha256": expected["sha256"],
                    **{
                        field: expected[field]
                        for field in (
                            "dependencies",
                            "homepage",
                            "dependency_versions",
                            "native_injector",
                        )
                        if field in expected
                    },
                },
                name,
            )
            self.assertEqual(len(pin), 64)

        git_manifest = json.loads(
            (self.out / "mods" / GAME / "git-mod.json").read_text(encoding="utf-8")
        )
        self.assertEqual(
            git_manifest,
            {
                "schema_version": 2,
                "id": "git-mod",
                "game": GAME,
                "name": "Git Mod",
                "version": "2.0",
                "sources": [
                    {"kind": "git", "repository": "example/git-mod", "commit": COMMIT}
                ],
                "redistributable": False,
                "dependencies": ["manual-mod"],
                "homepage": "https://example.org/git-mod",
            },
        )

        provenance = self.provenance_text()
        self.assertIn("## Rehash to contract v2", provenance)
        self.assertIn("converted 5 packages", provenance)
        self.assertRegex(provenance, r"on \d{4}-\d{2}-\d{2} \(UTC\)")
        for name in ("https-mod", "release-mod", "manual-mod", "manual-mod@0.9"):
            expected = self.fixture.expected[name]
            self.assertIn(f"`{GAME}/{name}`", provenance)
            self.assertIn(f"`{expected['sha256']}`", provenance)
        self.assertIn(f"`{GAME}/git-mod` | git source | git commit pin |", provenance)
        self.assertIn("Old pin table.", provenance)
        self.assertFalse(list(self.out.rglob("*.zip")))
        self.assertEqual(before, snapshot(self.fixture.bucket))
        self.assert_no_staging()

    def modn_hash(self, archive: Path) -> str:
        completed = subprocess.run(
            [str(MODN), "catalog", "hash", str(archive)],
            capture_output=True,
            text=True,
            encoding="utf-8",
            timeout=120,
            check=False,
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)
        return completed.stdout.strip()

    @NEEDS_MODN
    def test_empty_out_directory_is_accepted(self) -> None:
        self.out.mkdir()
        code, _, stderr = self.run_main(*self.convert_arguments())
        self.assertEqual(code, 0, stderr)
        self.assertTrue((self.out / "mods" / GAME / "https-mod.json").is_file())
        self.assert_no_staging()

    @NEEDS_MODN
    def test_converted_tree_loads_as_a_local_bucket(self) -> None:
        code, _, stderr = self.run_main(*self.convert_arguments())
        self.assertEqual(code, 0, stderr)
        data = self.root / "data"
        official = data / "modn" / "buckets" / "main"
        official.mkdir(parents=True)
        (official / "bucket.json").write_text(
            json.dumps({"schema_version": 2, "name": "main", "title": "Fixture Main"}),
            encoding="utf-8",
        )
        environment = {
            key: value for key, value in os.environ.items() if not key.startswith("MODN_")
        }
        environment.update(
            LOCALAPPDATA=str(data),
            MODN_CACHE=str(self.root / "cache"),
            XDG_STATE_HOME=str(self.root / "xdg"),
        )
        state = self.root / "state"
        added = subprocess.run(
            [str(MODN), "--json", "--state-dir", str(state), "bucket", "add", "fixture",
             str(self.out)],
            capture_output=True,
            text=True,
            encoding="utf-8",
            timeout=180,
            env=environment,
            check=False,
        )
        self.assertEqual(added.returncode, 0, added.stderr)
        self.assertEqual(json.loads(added.stdout)["name"], "fixture")
        listed = subprocess.run(
            [str(MODN), "--json", "--state-dir", str(state), "bucket", "list"],
            capture_output=True,
            text=True,
            encoding="utf-8",
            timeout=180,
            env=environment,
            check=False,
        )
        self.assertEqual(listed.returncode, 0, listed.stderr)
        names = [entry["name"] for entry in json.loads(listed.stdout)]
        self.assertIn("fixture", names)
        shown = subprocess.run(
            [str(MODN), "--json", "--state-dir", str(state), "bucket", "show",
             f"fixture/{GAME}/manual-mod"],
            capture_output=True,
            text=True,
            encoding="utf-8",
            timeout=180,
            env=environment,
            check=False,
        )
        self.assertEqual(shown.returncode, 0, shown.stderr)
        self.assertEqual(json.loads(shown.stdout)["version"], "1.0")


class CheckTests(RehashTestCase):
    def test_check_writes_nothing_and_passes_when_every_pin_matches(self) -> None:
        before = snapshot(self.fixture.bucket)
        code, stdout, stderr = self.run_main(
            self.fixture.bucket, "--archive-dir", self.fixture.archives, "--check"
        )
        self.assertEqual(code, 0, stderr)
        self.assertIn("Verified 5 packages", stdout)
        self.assertFalse(self.out.exists())
        self.assertEqual(before, snapshot(self.fixture.bucket))
        self.assert_no_staging()

    def test_check_fails_on_a_tampered_archive_and_writes_nothing(self) -> None:
        tampered = self.fixture.archives / "Manual Mod 1.0.zip"
        with tampered.open("ab") as handle:
            handle.write(b"tampered")
        actual = sha256(tampered)
        before = snapshot(self.fixture.bucket)
        code, _, stderr = self.run_main(
            self.fixture.bucket, "--archive-dir", self.fixture.archives, "--check"
        )
        self.assert_clean_failure(
            code,
            stderr,
            [
                f"{GAME}/manual-mod",
                f"expected {self.fixture.expected['manual-mod']['sha256']}",
                f"actual {actual}",
            ],
            before,
        )


from test_rehash_main_safety import *  # noqa: E402,F401,F403
from test_rehash_main_safety_failures import *  # noqa: E402,F401,F403


if __name__ == "__main__":
    unittest.main()
