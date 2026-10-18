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
