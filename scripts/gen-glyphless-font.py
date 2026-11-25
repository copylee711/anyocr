"""生成 GlyphLess 字体：只有 .notdef 和一个字形（宽 500/1000 em），用于 PDF 不可见文字层。

1 号字形是覆盖整个字符格（0,0 到 500,1000）的矩形轮廓。文字以渲染模式 3（不可见）绘制，轮廓永远不会显示；
但 PDFium（Chrome / Edge 的 PDF 引擎）按字形轮廓计算旋转文字的字符框，空轮廓会导致倾斜行的文字被丢弃。

思路同 Tesseract / OCRmyPDF 的 GlyphLessFont：所有 CID 都通过 CIDToGIDMap 映射到 1 号字形，
文字内容只由 ToUnicode CMap 决定，因此字体极小（不足 1 KB），且对任何语言都适用。

用法：uv run --with fonttools python -I scripts/gen-glyphless-font.py crates/anyocr-pdf/assets/glyphless.ttf
"""

import sys

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen


def main(out: str) -> None:
    upm = 1000
    glyphs = [".notdef", "glyph1"]
    fb = FontBuilder(upm, isTTF=True)
    fb.setupGlyphOrder(glyphs)
    fb.setupCharacterMap({})
    empty = TTGlyphPen(None).glyph()
    pen = TTGlyphPen(None)
    pen.moveTo((0, 0))
    pen.lineTo((0, 1000))
    pen.lineTo((500, 1000))
    pen.lineTo((500, 0))
    pen.closePath()
    fb.setupGlyf({".notdef": empty, "glyph1": pen.glyph()})
    fb.setupHorizontalMetrics({g: (500, 0) for g in glyphs})
    fb.setupHorizontalHeader(ascent=1000, descent=0)
    fb.setupNameTable({"familyName": "GlyphLessFont", "styleName": "Regular"})
    fb.setupOS2(sTypoAscender=1000, sTypoDescender=0, usWinAscent=1000, usWinDescent=0)
    fb.setupPost()
    fb.save(out)


if __name__ == "__main__":
    main(sys.argv[1])
