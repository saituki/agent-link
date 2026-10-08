"""設定の読み書きと住所の検査。"""

from __future__ import annotations

import json
import os
import stat
import tempfile
import unittest
from pathlib import Path

from sai_prep import config as config_mod


class AllowedServerUrlTest(unittest.TestCase):
    def test_accepts_https_and_localhost_http(self):
        for url in (
            "https://sai.example.ts.net",
            "https://sai.example.ts.net/",
            "http://127.0.0.1:8200",
            "http://localhost:8200",
            "http://127.0.0.1",
        ):
            with self.subTest(url=url):
                self.assertTrue(config_mod.allowed_server_url(url))

    def test_rejects_plain_http_to_other_hosts(self):
        for url in (
            "http://example.com",
            "http://192.168.0.1",
            "ftp://example.com",
            "example.com",
            "",
        ):
            with self.subTest(url=url):
                self.assertFalse(config_mod.allowed_server_url(url))


class ConfigIoTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.path = Path(self.tmp.name) / "config.json"
        self._env = os.environ.get("SAI_PREP_CONFIG")
        os.environ["SAI_PREP_CONFIG"] = str(self.path)
        self.addCleanup(self._restore_env)

    def _restore_env(self):
        if self._env is None:
            os.environ.pop("SAI_PREP_CONFIG", None)
        else:
            os.environ["SAI_PREP_CONFIG"] = self._env

    def test_defaults(self):
        config = config_mod.load()
        self.assertEqual(config.mlx_model, config_mod.DEFAULT_MLX_MODEL)
        self.assertEqual(config.mlx_port, 8766)
        self.assertTrue(config.translate)

    def test_roundtrip_and_permissions(self):
        config = config_mod.Config(server_url="https://sai.example.ts.net", token="sai_abc")
        config_mod.save(config)
        self.assertTrue(self.path.is_file())
        mode = stat.S_IMODE(self.path.stat().st_mode)
        self.assertEqual(mode, 0o600)
        loaded = config_mod.load()
        self.assertEqual(loaded.server_url, "https://sai.example.ts.net")
        self.assertEqual(loaded.token, "sai_abc")

    def test_unknown_keys_are_ignored(self):
        self.path.write_text(json.dumps({"server_url": "https://x", "unknown": 1}), encoding="utf-8")
        loaded = config_mod.load()
        self.assertEqual(loaded.server_url, "https://x")

    def test_broken_file_falls_back_to_defaults(self):
        self.path.write_text("not json", encoding="utf-8")
        self.assertEqual(config_mod.load().server_url, "")


if __name__ == "__main__":
    unittest.main()
