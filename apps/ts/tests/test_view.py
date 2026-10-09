"""画面の状態（C6）の保存と検査。"""

import os
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

from sai_ts import view  # noqa: E402


class TempProject:
    """setUp で一時フォルダを作って SAI_PROJECT_ROOT に入れ、tearDown で戻して消す。"""

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp()).resolve()
        self._saved = {k: os.environ.pop(k, None) for k in ("SAI_PROJECT_ROOT", "SAI_TS_TOKEN")}
        os.environ["SAI_PROJECT_ROOT"] = str(self.tmp)
        (self.tmp / "ts").mkdir()

    def tearDown(self):
        for k in ("SAI_PROJECT_ROOT", "SAI_TS_TOKEN"):
            os.environ.pop(k, None)
            if self._saved[k] is not None:
                os.environ[k] = self._saved[k]
        shutil.rmtree(self.tmp, ignore_errors=True)


class ViewTest(TempProject, unittest.TestCase):
    def test_empty_then_save_and_merge(self):
        self.assertEqual(view.load_view(), {})
        v = view.save_view({"dataset": "demo", "record": "r01", "t0": 10.0, "t1": 20.0,
                            "vars": ["ch01", "ch02"], "hidden_categories": ["noise"]}, "human")
        self.assertEqual((v["updated_by"], v["record"]), ("human", "r01"))
        self.assertTrue(v["updated_at"].endswith("Z"))
        self.assertEqual(view.load_view(), v)

        # 足りない項目は前の値を残す
        w = view.save_view({"selection": {"t0": 12.0, "t1": 13.5, "vars": ["ch01"]},
                            "selected_events": ["e-3f9a1c2b7d10"]}, "agent")
        self.assertEqual((w["t0"], w["t1"], w["vars"], w["updated_by"]), (10.0, 20.0, ["ch01", "ch02"], "agent"))
        self.assertEqual(w["selection"]["vars"], ["ch01"])
        # 選択を外す
        self.assertIsNone(view.save_view({"selection": None}, "human")["selection"])

    def test_record_change_clears_selection(self):  # 宿題 2
        view.save_view({"dataset": "demo", "record": "r02", "t0": 8.0, "t1": 12.0,
                        "selection": {"t0": 8.81, "t1": 10, "vars": ["ch02"]}}, "human")

        # 記録が変わり selection が無い → 選択は消える（別の記録の区間と読まれない）
        v = view.save_view({"record": "r03"}, "human")
        self.assertIsNone(v["selection"])
        self.assertEqual((v["record"], v["t0"], v["t1"]), ("r03", 8.0, 12.0))

        # 記録を変えて selection も送れば、送った値
        w = view.save_view({"record": "r01", "selection": {"t0": 1.0, "t1": 2.0}}, "agent")
        self.assertEqual(w["selection"], {"t0": 1.0, "t1": 2.0})

        # 記録が同じなら selection は残る
        self.assertEqual(view.save_view({"record": "r01", "t0": 0.0, "t1": 5.0}, "human")["selection"],
                         {"t0": 1.0, "t1": 2.0})

        # dataset が変わったときも消す
        self.assertIsNone(view.save_view({"dataset": "other"}, "human")["selection"])

    def test_server_keys_ignored_and_unknown_rejected(self):
        v = view.save_view({"updated_by": "agent", "updated_at": "x", "record": "r02"}, "human")
        self.assertEqual(v["updated_by"], "human")
        with self.assertRaises(ValueError):
            view.save_view({"bogus": 1}, "human")
        with self.assertRaises(ValueError):
            view.save_view({}, "robot")

    def test_invalid_values_rejected_and_file_untouched(self):
        view.save_view({"dataset": "demo", "t0": 1.0, "t1": 2.0}, "human")
        before = view.load_view()
        bad = [
            {"dataset": "../x"},
            {"record": "a b"},
            {"t0": 5.0, "t1": 5.0},
            {"t0": 5.0},                              # t0 だけ
            {"t0": float("nan"), "t1": 3.0},
            {"vars": "ch01"},                         # 配列でない
            {"vars": ["ok", "../bad"]},
            {"hidden_categories": [3]},
            {"selection": {"t0": 2.0, "t1": 1.0, "vars": []}},
            {"selection": "x"},
            {"selected_events": [None]},
        ]
        for patch in bad:
            with self.subTest(patch=patch), self.assertRaises(ValueError):
                view.save_view(patch, "human")
        self.assertEqual(view.load_view(), before)


if __name__ == "__main__":
    unittest.main()
