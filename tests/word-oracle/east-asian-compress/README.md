# Word 縦中横の vertCompress 比較

2026-11-07、Microsoft Word for Mac 16.114.3で確認。
`make-probe.py` は本文の tbRl / tbRlV と vertCompress 未指定・0・1を組み合わせた6ページを生成する。Wordで開き、PDFとして保存し、Poppler `pdftotext -bbox` の座標を `word-observation.json` に記録した。PDFの1ページ目も画像化して目視確認した。

各方向で3条件の座標が完全に一致する。10桁の数字の幅は約66.742ptで、12ptの1文字幅には縮小されない。Wordが属性を無視するというMS-OI29500 §2.1.75の記述とも一致する。

https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oi29500/efbc3ed1-1349-4572-93a0-a4e0f71e2a8b

この実測は本文とMac版Wordに限定される。表はJadeの共有描画経路の回帰テストでカバーし、Wordでの表の実測とは区別する。PDF自体は生成物としてリポジトリに保存していない。

`vertical-section-origins.docx` は `python3 make-section-probe.py` で再生成できる。
本文の `tbRl`、`tbRlV`、`tbLrV` を各9ページずつ配置し、左右非対称の余白、行グリッド、文字サイズ、縦中横の桁数を比較する。各ページの2列目「次列」で列が進む向きを確認できる。

同じスクリプトが、5種類のグリッド間隔を比較する40ページ、固定列間隔を比較する12ページ、空段落や折り返しを含む12ページも生成する。これら4件の基準座標はWord for Mac 16.114.4から採取し、macOSブラウザーCIで照合する。

既存の6ページを PyMuPDF の `get_texttrace()` で再確認すると、Wordは両方向とも漢字と縦中横の数字を正立させている。「前」の文字原点は `tbRl` が (483.600, 82.559) pt、`tbRlV` が (99.120, 82.559) pt。「年」と「後」のY原点はそれぞれ約108.364 pt、134.157 ptで、24 ptの等間隔にはならない。

元の6ページは、macOSブラウザーCIの `east-asian-compress-browser-mac` でも比較する。漢字と縦中横の数字を1文字ずつ、Word PDFの文字原点と各軸0.65 pt以内で照合する。
