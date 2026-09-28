"use strict";
// U20 が書くファイル（実装計画 U19 が先に空のファイルを作る）。
// 仕分けの画面（#/papers/triage）と、項目の板の中の操作を出す。
// 使う関数の決まりは共有契約 C-14:
//   seOpenAction(kind, nodeId, detail, target)
//   seOpenCard(nodeId, suggestion)
//   seProposalRow(proposal, onDone) → HTMLElement
//   seOpenTriage()
// 一番外側の名前は se で始める。
