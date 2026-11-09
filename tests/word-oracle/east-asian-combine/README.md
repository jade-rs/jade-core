# Word の組文字比較（2026-11-07）

Microsoft Word for Mac 16.114.3 で各 DOCX を開き、名前を付けて保存 → PDF →「印刷に最適」で出力。Poppler `pdftotext -bbox` で座標を取得し、`pdftoppm` の画像も目視確認した。入力は `python3 make-probe.py` で再生成できる。PDF はリポジトリに含めない。

- `combine-brackets.docx`: none / round / square / angle / curly は、それぞれ括弧なし / () / [] / <> / {}。4文字は6ptの2段、幅12pt。現在の括弧対応と一致。
- `combine-fit.docx`: combine 単独では6ptの2段。fitText（960 twips=48pt）との併用では通常の12ptの1段になり、組文字の指定は無効。本文で実機確認、表内は同じ優先順位を描画回帰テストで検証。
- 1文字の combine は6ptで表示。PDF の text matrix を確認すると、通常文字と同じベースライン（y=202）だった。初期の「下段」という目視による解釈を訂正し、半サイズ・ベースライン一致に修正。
- 複数文字は通常の12ptの基準線から上段が6.24pt上、下段が2.4pt下。括弧は通常の基準線に一致。`word-observation.json` に外側 CTM と text matrix の値も保存。描画オフセットをこの実測に合わせて修正。

`word-observation.json` に PDF の語単位座標を保存。異なるラベルのため行全体の幅は直接比較しない。

追加の `combine-scale.docx` は文字幅倍率の比較用。1・3・4文字 × 括弧なし／丸括弧 × 50%／100%／200%の18条件を含む。Macがロックされていたため、こちらのWord実機出力は未取得で、`word-observation.json` の実測対象には含めていない。

`combine-table-scale.docx` は同じ18条件を表のセル内に置いた追加プローブ。Word実機出力は未取得。表の計測と組文字描画が本文と異なる問題を、本文／表内 × 18条件の描画回帰テストで検出した。

`combine-spacing.docx` は文字間隔 −0.75pt／1.5pt × 既存の18条件を含む36段落。文字間隔を含む組文字の計測・描画の比較用で、Word実機出力は未取得。本文／表内の描画は自動回帰テストで検証する。

`combine-vertical-fit.docx` は縦書きセル tbRl／tbRlV それぞれで fitText 単独・組文字併用・縦中横併用・両方併用を比較する8行。Microsoftの優先順位仕様に基づく修正の比較用で、Word実機出力は未取得。
