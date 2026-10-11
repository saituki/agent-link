"""イベントの追加・変更・削除・検査・並行書き込み・TSV（AC1〜AC4）と、スキーマの保存。"""

import json
import os
import shutil
import sys
import tempfile
import threading
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

from sai_ts import demo, events  # noqa: E402
from sai_ts import paths  # noqa: E402

SRC = "human"


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


def area(**kw):
    return {"type": "area", "record": "r01", "vars": ["ch01"], "t0": 1.0, "t1": 2.0,
            "category": "burst", "text": "", "metrics": {}, **kw}


def line(**kw):
    return {"type": "line", "record": "r01", "vars": [], "t": 3.0,
            "category": "noise", "text": "", "metrics": {}, **kw}


class EventsTest(TempProject, unittest.TestCase):
    def setUp(self):
        super().setUp()
        # 記録は 10 秒（srate 500・5000 サンプル）。カテゴリは burst・noise、メトリクスは amp（0〜100）
        demo.make(self.tmp / "ts" / "datasets" / "demo", records=2, nvars=2, seconds=10.0, srate=500.0)
        self.ds = "demo"

    def test_add_update_delete_keeps_latest(self):
        a = events.add_event(self.ds, area(text="最初"), SRC)
        self.assertEqual((a["rev"], a["op"], a["source"]), (1, "put", "human"))
        self.assertTrue(a["id"].startswith("e-") and len(a["id"]) == 14)
        self.assertTrue(a["created_at"].endswith("Z"))

        b = events.update_event(self.ds, a["id"], {"text": "直した", "id": "x", "rev": 9}, "agent")
        self.assertEqual((b["id"], b["rev"], b["text"], b["source"]), (a["id"], 2, "直した", "agent"))
        self.assertEqual([e["text"] for e in events.list_events(self.ds)], ["直した"])

        events.delete_event(self.ds, a["id"], SRC)
        self.assertEqual(events.list_events(self.ds), [])
        with self.assertRaises(KeyError):
            events.update_event(self.ds, a["id"], {"text": "x"}, SRC)
        with self.assertRaises(KeyError):
            events.delete_event(self.ds, a["id"], SRC)

        raw = (self.tmp / "ts/datasets/demo/events.jsonl").read_text(encoding="utf-8").splitlines()
        self.assertEqual([json.loads(x)["rev"] for x in raw], [1, 2, 3])
        self.assertEqual(json.loads(raw[-1])["op"], "del")

    def test_filters_and_order(self):
        events.add_events(self.ds, [
            line(record="r02", t=1.0),
            area(record="r01", t0=5.0, t1=6.0, category="noise"),
            line(record="r01", t=2.5, category="burst"),
            area(record="r01", t0=1.0, t1=2.0),
        ], SRC)
        got = [(e["record"], e["type"]) for e in events.list_events(self.ds)]
        self.assertEqual(got, [("r01", "area"), ("r01", "line"), ("r01", "area"), ("r02", "line")])
        self.assertEqual([e["t0"] if e["type"] == "area" else e["t"]
                          for e in events.list_events(self.ds, record="r01")], [1.0, 2.5, 5.0])
        self.assertEqual(len(events.list_events(self.ds, category="noise")), 2)
        self.assertEqual(len(events.list_events(self.ds, type="line")), 2)
        self.assertEqual(len(events.list_events(self.ds, record="r01", category="burst", type="line")), 1)
        with self.assertRaises(ValueError):
            events.list_events(self.ds, type="point")

    def test_validation_errors_write_nothing(self):
        events.add_event(self.ds, area(), SRC)  # 既に 1 件ある
        before = (self.tmp / "ts/datasets/demo/events.jsonl").read_bytes()
        bad = [
            area(category="nope"),                      # 無いカテゴリ
            area(vars=["ch99"]),                        # 無い変数
            area(t0=2.0, t1=2.0),                       # t0 >= t1
            area(t0=-1.0),                              # 範囲外
            area(t1=10.5),                              # 記録より後
            line(t=10.5),                               # 範囲外（line）
            area(metrics={"amp": "強い"}),              # 型が違う
            area(metrics={"amp": 101}),                 # 範囲外
            area(metrics={"amp": True}),                # bool は数ではない
            area(metrics={"kind": "emg"}),              # 無いメトリクス
            area(text="x" * 20001),                     # 長すぎる
            area(type="point"),                         # 型が違う
            area(category=["burst"]),                   # 形が違う（リスト）
            area(record="r99"),                         # 無い記録
            area(bogus=1),                              # 知らない項目
        ]
        for ev in bad:
            with self.subTest(ev=ev), self.assertRaises(ValueError):
                events.add_event(self.ds, ev, SRC)
        # enum と integer の検査（スキーマを足してから）
        events.save_schema(self.ds, {"categories": [{"name": "burst", "color": "#000000"}],
                                     "metrics": [{"name": "kind", "type": "enum", "values": ["emg"]},
                                                 {"name": "n", "type": "integer", "min": 0, "max": 5}]})
        for ev in [area(metrics={"kind": "move"}), area(metrics={"n": 1.5}), area(metrics={"n": 6})]:
            with self.subTest(ev=ev), self.assertRaises(ValueError):
                events.add_event(self.ds, ev, SRC)
        self.assertEqual(before, (self.tmp / "ts/datasets/demo/events.jsonl").read_bytes())

        # 1 件でも誤りがあれば、正しい件も書かない
        with self.assertRaises(ValueError):
            events.add_events(self.ds, [area(), area(category="nope")], SRC)
        self.assertEqual(len(events.list_events(self.ds)), 1)

    def test_valid_metrics_saved_and_normalized(self):
        ev = events.add_event(self.ds, area(metrics={"amp": 3.2}, vars=[]), SRC)
        self.assertEqual(ev["metrics"], {"amp": 3.2})
        self.assertEqual(ev["vars"], [])
        self.assertNotIn("t", ev)
        self.assertNotIn("t0", events.add_event(self.ds, line(), SRC))

    def test_update_type_change_drops_unused_time_keys(self):
        a = events.add_event(self.ds, line(), SRC)
        b = events.update_event(self.ds, a["id"], {"type": "area", "t0": 4.0, "t1": 4.5}, SRC)
        self.assertEqual((b["type"], b["t0"], b["t1"]), ("area", 4.0, 4.5))
        self.assertNotIn("t", b)
        with self.assertRaises(ValueError):  # t を足さずに line にする
            events.update_event(self.ds, b["id"], {"type": "line"}, SRC)
        c = events.add_event(self.ds, line(), SRC)
        with self.assertRaises(ValueError):  # t0・t1 を足さずに area にする
            events.update_event(self.ds, c["id"], {"type": "area"}, SRC)

    def test_concurrent_appends_no_broken_lines(self):
        errors = []

        def worker(k):
            try:
                for i in range(200):
                    events.add_event(self.ds, line(text=f"{k}-{i}"), SRC)
            except Exception as e:  # noqa: BLE001 - スレッドの中の失敗を表に出す
                errors.append(e)

        ts = [threading.Thread(target=worker, args=(k,)) for k in range(2)]
        for t in ts:
            t.start()
        for t in ts:
            t.join()
        self.assertEqual(errors, [])
        raw = (self.tmp / "ts/datasets/demo/events.jsonl").read_text(encoding="utf-8").splitlines()
        self.assertEqual(len(raw), 400)
        self.assertTrue(all(json.loads(x)["op"] == "put" for x in raw))
        self.assertEqual(len(events.list_events(self.ds)), 400)

    def test_broken_line_skipped_and_next_append_survives(self):
        a = events.add_event(self.ds, line(), SRC)
        path = self.tmp / "ts/datasets/demo/events.jsonl"
        with open(path, "a", encoding="utf-8") as f:
            f.write('{"op": "put", "id": "e-broken')  # 書きかけで止まった行（改行なし）
        self.assertEqual([e["id"] for e in events.list_events(self.ds)], [a["id"]])
        b = events.add_event(self.ds, line(), SRC)
        self.assertEqual({e["id"] for e in events.list_events(self.ds)}, {a["id"], b["id"]})

    def test_text_with_unicode_line_separator_roundtrips(self):
        a = events.add_event(self.ds, line(text="a b\tc"), SRC)
        self.assertEqual(events.list_events(self.ds)[0]["text"], "a b\tc")
        self.assertEqual(a["text"], "a b\tc")

    def test_export_tsv(self):
        events.save_schema(self.ds, {"categories": [{"name": "burst", "color": "#d9534f"},
                                                    {"name": "noise", "color": "#6c757d"}],
                                     "metrics": [{"name": "amp", "type": "number"},
                                                 {"name": "ok", "type": "boolean"}]})
        events.add_events(self.ds, [
            area(t0=1.5, t1=2.25, vars=["ch01", "ch02"], text="a\tb\nc",
                 metrics={"amp": 3.2, "ok": True}),
            line(record="r01", t=0.5, category="noise", vars=[], text=""),
        ], SRC)
        out = events.export_tsv(self.ds).splitlines()
        head = out[0].split("\t")
        self.assertEqual(head, ["onset", "duration", "trial_type", "channel", "record", "text",
                                "source", "id", "m_amp", "m_ok"])
        rows = [dict(zip(head, r.split("\t"))) for r in out[1:]]
        self.assertEqual([r["onset"] for r in rows], ["0.500000", "1.500000"])
        self.assertEqual(rows[0]["duration"], "0.000000")
        self.assertEqual(rows[0]["channel"], "n/a")
        self.assertEqual(rows[0]["text"], "n/a")
        self.assertEqual(rows[0]["m_amp"], "n/a")
        self.assertEqual(rows[0]["m_ok"], "n/a")
        self.assertEqual(rows[1]["duration"], "0.750000")
        self.assertEqual(rows[1]["channel"], "ch01|ch02")
        self.assertEqual(rows[1]["text"], "a b c")
        self.assertEqual(rows[1]["m_amp"], "3.2")
        self.assertEqual(rows[1]["m_ok"], "true")
        self.assertEqual(rows[1]["source"], "human")
        self.assertTrue(all(len(r.split("\t")) == 10 for r in out))

    def test_schema_validation_and_used_category(self):
        good = {"categories": [{"name": "burst", "color": "#d9534f", "visible": True, "desc": "x"}],
                "metrics": [{"name": "amp", "type": "number", "min": 0, "max": 100}]}
        self.assertEqual(events.save_schema(self.ds, good)["categories"][0]["name"], "burst")
        self.assertEqual(events.load_schema(self.ds)["metrics"][0]["max"], 100)

        bad = [
            {"categories": [{"name": "a b", "color": "#000000"}]},
            {"categories": [{"name": "a", "color": "red"}]},
            {"categories": [{"name": "a", "color": "#000000"}, {"name": "a", "color": "#000000"}]},
            {"metrics": [{"name": "m", "type": "float"}]},
            {"metrics": [{"name": "m", "type": "enum", "values": []}]},
            {"metrics": [{"name": "m", "type": "number", "min": 5, "max": 1}]},
        ]
        for s in bad:
            with self.subTest(s=s), self.assertRaises(ValueError):
                events.save_schema(self.ds, s)

        events.add_event(self.ds, area(category="burst"), SRC)
        with self.assertRaises(ValueError) as cm:
            events.save_schema(self.ds, {"categories": [{"name": "noise", "color": "#000000"}]})
        self.assertEqual(str(cm.exception), "カテゴリ burst は 1 件のイベントが使っています")
        # 使われていないカテゴリなら消せる
        events.save_schema(self.ds, {"categories": [{"name": "burst", "color": "#000000"}]})

    def test_category_type_and_ch_checked(self):  # AC5（G1-C2）
        events.save_schema(self.ds, {"categories": [
            {"name": "HFO", "color": "#f59e0b", "type": "area", "ch": "single"},
            {"name": "sleep", "color": "#6c757d", "ch": "all"},
            {"name": "plain", "color": "#000000"},
        ]})
        with self.assertRaises(ValueError) as cm:
            events.add_event(self.ds, area(category="HFO", vars=["ch01", "ch02"]), SRC)
        self.assertEqual(str(cm.exception), "vars: カテゴリ HFO は ch を 1 つだけ選びます")
        with self.assertRaises(ValueError):
            events.add_event(self.ds, area(category="HFO", vars=[]), SRC)
        with self.assertRaises(ValueError) as cm:
            events.add_event(self.ds, line(category="HFO", vars=["ch01"]), SRC)
        self.assertEqual(str(cm.exception), "type: カテゴリ HFO は area です")
        with self.assertRaises(ValueError):
            events.add_event(self.ds, line(category="sleep", vars=["ch01"]), SRC)
        self.assertEqual(events.list_events(self.ds), [])

        ok = [area(category="HFO", vars=["ch02"]), line(category="sleep", vars=[]), line(category="plain", vars=["ch01", "ch02"])]
        self.assertEqual(len(events.add_events(self.ds, ok, SRC)), 3)

        # update でも同じ決まりで検査する
        e = events.list_events(self.ds, category="HFO")[0]
        with self.assertRaises(ValueError):
            events.update_event(self.ds, e["id"], {"vars": ["ch01", "ch02"]}, SRC)
        with self.assertRaises(ValueError):
            events.update_event(self.ds, e["id"], {"type": "line", "t": 1.0}, SRC)
        self.assertEqual(events.list_events(self.ds, category="HFO")[0]["rev"], 1)

    def test_category_type_ch_values_checked_in_schema(self):
        for c in ({"name": "a", "color": "#000000", "type": "point"},
                  {"name": "a", "color": "#000000", "ch": "many"}):
            with self.subTest(c=c), self.assertRaises(ValueError):
                events.save_schema(self.ds, {"categories": [c]})
        s = events.save_schema(self.ds, {"categories": [{"name": "a", "color": "#000000", "type": "line"}]})
        self.assertEqual(s["categories"][0]["type"], "line")
        self.assertEqual(events.load_schema(self.ds)["categories"][0]["type"], "line")

    def test_missing_dataset_is_key_error(self):
        with self.assertRaises(KeyError):
            events.list_events("nope")
        with self.assertRaises(ValueError):
            events.add_event("../x", area(), SRC)


if __name__ == "__main__":
    unittest.main()
