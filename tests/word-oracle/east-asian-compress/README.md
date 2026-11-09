# Word 縦中横の vertCompress 比較

2026-11-07、Microsoft Word for Mac 16.114.3で確認。
`make-probe.py` は本文の tbRl / tbRlV と vertCompress 未指定・0・1を組み合わせた6ページを生成する。Wordで開き、PDFとして保存し、Poppler `pdftotext -bbox` の座標を `word-observation.json` に記録した。PDFの1ページ目も画像化して目視確認した。

各方向で3条件の座標が完全に一致する。10桁の数字の幅は約66.742ptで、12ptの1文字幅には縮小されない。Wordが属性を無視するというMS-OI29500 §2.1.75の記述とも一致する。

https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oi29500/efbc3ed1-1349-4572-93a0-a4e0f71e2a8b

この実測は本文とMac版Wordに限定される。表はJadeの共有描画経路の回帰テストでカバーし、Wordでの表の実測とは区別する。PDF自体は生成物としてリポジトリに保存していない。
