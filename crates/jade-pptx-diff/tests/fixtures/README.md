# テスト用のプレゼンテーション

`deck-old.pptx` と `deck-new.pptx` は python-pptx 1.0.2 の既定テンプレートから
生成したペア。中身は次のとおりで、「スライドを 1 枚差し込んでも以降がズレない」
ことと、段落・表セル単位の差分を確認するために使う。

| | old | new |
| --- | --- | --- |
| 1 | 四半期報告 / 2026 Q2 | 四半期報告 / 2026 Q2 (改訂) |
| 2 | 売上 (東京 120 / 大阪 80) | **サマリ (前年比 +12%)** ← 挿入 |
| 3 | 内訳 (表: 商品 200) | 売上 (東京 **150** / 大阪 80 / **名古屋 40**) |
| 4 | — | 内訳 (表: 商品 **260**) |

生成に使ったスクリプトは `tests/deck.rs` の doc コメントに残してある。
差し替えるときは同じ構成を保つこと (テストが件数を直接見ている)。

## shift-old.pptx / shift-new.pptx

同じ内容のテキストボックスが 2 つ載ったスライド 1 枚。新しいほうでは片方を
左から 1.0 インチ → 3.5 インチへ動かしてある。中身は一字も変えていないので、
「位置だけが変わった図形」を要約で拾えているかの確認に使う。

```python
from pptx import Presentation
from pptx.util import Inches

for path, left in [("shift-old.pptx", Inches(1.0)), ("shift-new.pptx", Inches(3.5))]:
    prs = Presentation()
    prs.slide_width, prs.slide_height = Inches(13.333), Inches(7.5)
    s = prs.slides.add_slide(prs.slide_layouts[6])
    s.shapes.add_textbox(left, Inches(2.0), Inches(4), Inches(1)).text_frame.text = "動かしただけの箱"
    s.shapes.add_textbox(Inches(8), Inches(2.0), Inches(4), Inches(1)).text_frame.text = "動かない箱"
    prs.save(path)
```


## colors-transformed / colors-linear / colors-wrong-srgb

`generate_colors.py`が標準ライブラリだけで生成する、文字・画像のない4つの四角形。最初の2図形は白の50% shadeと黒の50% tint、同じ見た目のRGB188、異なるRGB128のいずれか。後半の直接指定2色、図形ID、配置、寸法は同じ。独立した描画と全画素の検証条件は [docs/pptx-color-reference.md](../../../../docs/pptx-color-reference.md) を参照。描画を見ずに比較器自身の出力を正解としていない。

## colors-chain-* / colors-alpha-*

同じ生成器で連続するshade、連続するtint、shadeからtint、alphaMod、alphaとalphaModの組合せ、shadeの間の透明度指定を検証する。`precise`は途中で8bitへ丸めない計算の直接指定色、`rounded`は各操作で丸めた誤った描画を区別する対照。`alpha-explicit`は半透明の明示指定、`alpha-opaque`は不透明の対照。`chain-alpha-precise`では最後の対照図形だけをRGB188から189へ変え、同じ見た目の最初の2図形を変更扱いしないことも確認する。入力ハッシュ、独立した透明度の全画素検証、標準sRGBとLibreOfficeで異なる色の未完了範囲は上記の記録を参照。

## colors-unary-*

val属性を持たないcomp・inv・gray・gamma・invGammaと、gamma/invGammaの復元・補色・連続shade・半透明の組合せを検証する。`transformed`はRGB18,52,86とRGB128への変換、`precise`は標準sRGBモデルの最終RGBを直接指定、`base`は変換前の対照。`ordered-precise`は最後の対照図形だけをRGB188から189へ変える。補色は独立したLibreOfficeDev出力との全画素差分0を確認した。他の4変換には独立描画との値の差もあり、全原本一致を証明する資料とはしない。[条件・期待値・ハッシュ](../../../../docs/pptx-color-reference.md)と[独立描画の測定JSON](../../../../docs/pptx-unary-reference.json)を参照。

`generate_color_pipeline.py` は RGB 成分の絶対値・倍率・加算、HSL、scRGB、
テーマ継承・透明度と色キャッシュの 16 組を生成する（各 transformed/precise/base の 3 ファイル）。
`color-pipeline-goldens.json` の RGB8 は sRGB 伝達関数と Python `colorsys` の
HLS 計算から別途求めた固定値で、アプリの描画出力から採取していない。
RGB 成分は線形光で変換し、HSL は符号化 sRGB 上で変換する。各操作の範囲制限を
適用した後、出力時にだけ 8bit へ丸める。`hsl-chain` は途中で丸めると RGB17 に
なる RGB18 の例、`scrgb-chain` と `hsl-base-chain` は初期色の丸めも検出する。
テーマ用ファイルは slide/layout/master/theme の関係を含む。透明度の画面値は
白背景上の Chromium/WebKit の合成結果であり、Office 全紙面の正解画像ではない。

`hsl`・`scrgb`・`system` は変換を持たない異なる2色を同じスライドへ並べ、
val 以外の hue/sat/lum、r/g/b、lastClr を無視したキャッシュの衝突も検出する。

`alpha-precision` は不透明度（alpha）50.39%の黒・赤を白背景に描く。独立した
LibreOfficeDev/Popplerの中心RGBAは127,127,127 / 255,127,127である。
透明度を小数3桁へ丸めるとブラウザーの画素が1階調ずれることを検査する。
