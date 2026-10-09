# sai-ts（時系列の確認と判断の道具）

人が時系列を見て判断し、その判断をイベントとして LLM agent に渡すための道具。
段階 A は Mac で直接動かす（`127.0.0.1:8210`）。

## 入れ方と起動

```bash
cd apps/ts && uv sync          # .venv を作る（fastapi・uvicorn・numpy）
cd ~/workdir/Sai/sai-tree      # .sai/ のあるプロジェクトの中
apps/ts/bin/sai-ts demo demo   # 合成データ（ts/datasets/demo）を作る
apps/ts/bin/sai-ts serve       # http://127.0.0.1:8210/ を開く
```

## 環境変数

| 名前 | 既定 | 中身 |
|---|---|---|
| `SAI_PROJECT_ROOT` | cwd から `.sai/` をたどる | プロジェクトの場所 |
| `SAI_TS_PORT` | `8210` | サーバの番号 |
| `SAI_TS_TOKEN` | 空（認証なし） | 設定すると API と `/ext/` は `Authorization: Bearer` か `?token=` を要する |
| `SAI_LINK_BIN` | `sai-link` | チャットが呼ぶ sai-link |
| `SAI_TS_PYTHON` | 実行中の Python | スクリプトと拡張の server.py を動かす Python |

置き場は `<project>/ts/` の中だけ。記録は `datasets/<ds>/records/<rec>/meta.json` と `<var>.f32`（float32・リトルエンディアン・ヘッダ無し）。

## 試験

```bash
cd apps/ts
for f in tests/test_*.py; do PYTHONPATH=src .venv/bin/python "$f" || exit 1; done
```

`test_server_data.py` は `httpx`（dev グループ）を使う。
