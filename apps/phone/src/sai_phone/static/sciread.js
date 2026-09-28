"use strict";
// U20 が書くファイル（実装計画 U19 が先に空のファイルを作る）。
// 読む札（#/papers/read/<run_id>）の画面。
// 使う関数の決まりは共有契約 C-14:
//   srOpen(params)            … params.runId
//   srOpenReadCard(nodeId, detail)
// 一番外側の名前は sr で始める。
