# Noto Sans Arabic

The font is distributed under the SIL Open Font License 1.1 in `OFL.txt`.
The variable source `NotoSansArabic[wdth,wght].ttf` comes from the Google
Fonts repository at [8b0a1d0](https://github.com/google/fonts/tree/8b0a1d0f5983c89bc2b93f1b5fb55f9e252744b5/ofl/notosansarabic).
The regular instance retains all glyphs and shaping tables at the source's
default weight and width.

```sh
fonttools varLib.instancer 'NotoSansArabic[wdth,wght].ttf' wght=400 wdth=100 \
  --static --no-recalc-timestamp -o NotoSansArabic-Regular.ttf
```

Generated with FontTools 4.61.1. SHA-256
`65efad4885c590b640d9601b8cc6d9b66026e9ff74154ac600207600842c0f70`.
