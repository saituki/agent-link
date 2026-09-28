"use strict";
// U21 が書くファイル（実装計画 U19 が先に空のファイルを作る）。
// 根の図（研究方針の木）と、候補の表。
// 使う関数の決まりは共有契約 C-14:
//   sgRenderGraph(container, status, onSelect)   … onSelect(nodeId)
//   sgRenderTable(container, detail)
// scitree.js（U19）は typeof で確かめてから呼ぶ。関数が無い間は
// 「この表示はまだありません」の 1 行を出す。
// 一番外側の名前は sg で始める。
