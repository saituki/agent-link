"""ch 設定（channels.json・AC1）。保存・既定の埋め込み・検査の誤り・ファイルが無いとき。"""

import copy
import os
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

from sai_ts import channels, demo  # noqa: E402

GOOD = {"version": 1, "channels": [
    {"name": "ch01", "label": "Fp1", "color": "#7dd3fc", "gain": 1.0,
     "spectrum": {"on": False, "fmin": 0, "fmax": 250, "win_ms": 250}},
    {"name": "ch02"},
]}


class TempProject:
    """setUp で一時フォルダを作って SAI_PROJECT_ROOT に入れ、tearDown で戻して消す。"""

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp()).resolve()
        self._saved = {k: os.environ.pop(k, None) for k in ("SAI_PROJECT_ROOT", "SAI_TS_TOKEN")}
        os.environ["SAI_PROJECT_ROOT"] = str(self.tmp)

    def tearDown(self):
        for k in ("SAI_PROJECT_ROOT", "SAI_TS_TOKEN"):
            os.environ.pop(k, None)
            if self._saved[k] is not None:
                os.environ[k] = self._saved[k]
        shutil.rmtree(self.tmp, ignore_errors=True)


class ChannelsTest(TempProject, unittest.TestCase):
    def setUp(self):
        super().setUp()
        demo.make(self.tmp / "ts" / "datasets" / "demo", records=1, nvars=2, seconds=1.0, srate=100.0)
        self.ds = "demo"

    def test_save_fills_defaults_and_roundtrips(self):
        got = channels.save_channels(self.ds, GOOD)
        self.assertEqual(got["version"], 1)
        self.assertEqual(got["channels"][1], {"name": "ch02", "label": "", "color": "", "gain": 1})
        self.assertEqual(got["channels"][0]["spectrum"], GOOD["channels"][0]["spectrum"])
        self.assertNotIn("spectrum", got["channels"][1])
        self.assertEqual(channels.load_channels(self.ds), got)

    def test_missing_file_is_empty(self):
        self.assertEqual(channels.load_channels(self.ds), {"version": 1, "channels": []})

    def test_bad_values_are_value_error(self):
        def item(**kw):
            c = {"name": "ch01", **kw}
            return {"version": 1, "channels": [c]}

        dup = {"version": 1, "channels": [{"name": "ch01"}, {"name": "ch01"}]}
        many = {"version": 1, "channels": [{"name": f"c{i}"} for i in range(513)]}
        bad = [
            [],                                              # object ではない
            {"channels": "x"},                               # 配列ではない
            {"version": 2, "channels": []},                  # 版の違い
            {"bogus": 1, "channels": []},                    # 知らない項目（上）
            dup,                                             # 名前の重なり
            many,                                            # 513 件
            {"channels": [{"name": "../x"}]},                # 名前の形
            {"channels": ["ch01"]},                          # 項目が object ではない
            item(gain=0),                                    # gain は 0 より大きい
            item(gain=1001),                                 # gain の上限
            item(gain=True),                                 # bool は数ではない
            item(color="red"),                               # color の形
            item(label="x" * 65),                            # label が長い
            item(bogus=1),                                   # 知らない項目
            item(spectrum={"on": True, "fmin": 0, "fmax": 0, "win_ms": 250}),    # fmax <= fmin
            item(spectrum={"on": "yes", "fmin": 0, "fmax": 250, "win_ms": 250}),  # on は bool
            item(spectrum={"on": True, "fmin": -1, "fmax": 250, "win_ms": 250}),  # fmin < 0
            item(spectrum={"on": True, "fmin": 0, "fmax": 250, "win_ms": 10001}),  # win_ms 上限
            item(spectrum={"on": True, "fmin": 0, "fmax": 250, "win_ms": 250, "x": 1}),  # spectrum の知らない項目
        ]
        for body in bad:
            with self.subTest(body=str(body)[:60]), self.assertRaises(ValueError):
                channels.save_channels(self.ds, body)
        self.assertFalse((self.tmp / "ts" / "datasets" / "demo" / channels.FILE).exists())

    def test_unknown_item_message(self):
        with self.assertRaises(ValueError) as cm:
            channels.save_channels(self.ds, {"channels": [{"name": "ch01", "visible": True}]})
        self.assertEqual(str(cm.exception), "visible は channels の項目ではありません")

    def test_gain_bounds_accepted(self):
        body = copy.deepcopy(GOOD)
        body["channels"][0]["gain"] = 1000
        body["channels"][0]["spectrum"]["win_ms"] = 10000
        self.assertEqual(channels.save_channels(self.ds, body)["channels"][0]["gain"], 1000)

    def test_missing_dataset_is_key_error(self):
        with self.assertRaises(KeyError):
            channels.load_channels("nope")
        with self.assertRaises(KeyError):
            channels.save_channels("nope", GOOD)


if __name__ == "__main__":
    unittest.main()
