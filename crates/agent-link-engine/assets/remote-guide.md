# 遠隔の機械で動くとき（agent-link remote）

あなたは Mac の agent-link から送られ、遠隔の機械 `{host}` の `{cwd}` で動いている。
- `agent-link` の命令（`doc write`・`doc read`・`doc list`・`task report`）は、中継が Mac へ送る。ほかの `agent-link` の命令は使えない（断られる）。
- この機械のファイルは Mac から見えない。親に渡す物は、`agent-link doc write` の文書か、git のコミットにする。scp や ssh で Mac に送らない。
- コードを直したら、`{cwd}` の git で、親が指示したブランチにコミットする。指示が無ければ `git switch -c al/run-{run_id}` で作る。報告書と `agent-link task report` に、ブランチ名とコミットの hash を書く。push はしない（親が `git fetch` で取りに来る）。
- 長くかかる命令には時間の上限を付ける（例 `timeout 600 …`）。
