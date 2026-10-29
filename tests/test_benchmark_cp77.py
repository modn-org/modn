"""Regression checks for the read-only CP77 ZIP metadata budget."""
from pathlib import Path
import runpy
import struct
import tempfile
import unittest
import zipfile

SCRIPT = Path(__file__).resolve().parents[1] / ".agents/skills/verify-modn/scripts/benchmark_cp77.py"
within_budget = runpy.run_path(str(SCRIPT))["zip_index_within_budget"]


class ZipIndexBudgetTest(unittest.TestCase):
    def test_normal_small_zip(self):
        with tempfile.TemporaryDirectory() as root:
            archive = Path(root) / "small.zip"
            with zipfile.ZipFile(archive, "w") as writer:
                writer.writestr("a.txt", "sample")
            self.assertTrue(within_budget(archive))

    def test_zip64_locator_with_misleading_regular_entry_count(self):
        with tempfile.TemporaryDirectory() as root:
            archive = Path(root) / "large.zip"
            with zipfile.ZipFile(archive, "w") as writer:
                for index in range(2050):
                    writer.writestr(f"{index:04}.txt", "")
            raw = archive.read_bytes()
            eocd = raw.rfind(b"PK\x05\x06")
            self.assertGreaterEqual(eocd, 0)
            central_bytes, central_offset = struct.unpack_from("<II", raw, eocd + 12)
            zip64_end = struct.pack(
                "<4sQHHIIQQQQ", b"PK\x06\x06", 44, 45, 45, 0, 0,
                2050, 2050, central_bytes, central_offset,
            )
            zip64_locator = struct.pack("<4sIQI", b"PK\x06\x07", 0, eocd, 1)
            ordinary_end = bytearray(raw[eocd:])
            struct.pack_into("<HHII", ordinary_end, 8, 1, 1, 42, central_offset)
            archive.write_bytes(raw[:eocd] + zip64_end + zip64_locator + ordinary_end)

            with zipfile.ZipFile(archive) as reader:
                self.assertEqual(len(reader.infolist()), 2050)
            self.assertFalse(within_budget(archive), "must inspect effective ZIP64 index")


if __name__ == "__main__":
    unittest.main()
