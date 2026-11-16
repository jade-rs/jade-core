"""Reproduce the six original Word half-cell exact-width controls."""
from pathlib import Path
import hashlib,re
from zipfile import ZipFile,ZipInfo,ZIP_DEFLATED
ROOT=Path(__file__).resolve().parents[2]
PARENT_SHA256={'16.5': '76a723d05dc4a08a30216828537e0b6677277b680f40371c15c69b4f8db8be2c', '25.5': '561a55419344d1e20b20ed648b9b76e7f7060439a2ed98011920d74bda279572'}

def generate(output):
    output.mkdir(parents=True,exist_ok=False)
    count=0
    for size,center in [(16.5,330.0),(25.5,510.0)]:
        token=str(size).replace('.','p')
        parent=ROOT/'tests/word-oracle/windows/observations/sources'/f'word-windows-cjk-cell-mincho-halfwidth-kana-size{token}-regular-compat15.docx'
        if hashlib.sha256(parent.read_bytes()).hexdigest()!=PARENT_SHA256[str(size)]:raise ValueError('Captured half-cell parent changed')
        with ZipFile(parent) as archive:
            for offset in (-0.1,0.0,0.1):
                width=center+offset
                doc=archive.read('word/document.xml').decode('utf-8')
                page=int(re.search(r'<w:pgSz w:w="([0-9]+)"',doc)[1]);left=int(re.search(r'<w:pgMar[^>]* w:left="([0-9]+)"',doc)[1])
                right=page-left-round(width*20)
                doc,changed=re.subn(r'(<w:pgMar[^>]* w:right=")[0-9]+',lambda match:match[1]+str(right),doc)
                if changed!=1:raise ValueError('Captured margins changed')
                width_token=('%g'%width).replace('.','p')
                name=f'word-windows-printer-halfwrap-size{token}-width{width_token}-compat15.docx'
                with ZipFile(output/name,'x',compression=ZIP_DEFLATED) as target:
                    for old in archive.infolist():
                        info=ZipInfo(old.filename,(2026,11,16,0,0,0));info.create_system=0;info.compress_type=ZIP_DEFLATED;info.external_attr=old.external_attr
                        target.writestr(info,doc.encode('utf-8') if old.filename=='word/document.xml' else archive.read(old.filename))
                count+=1
    return count


if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    print(f"Generated {generate(parser.parse_args().output)} half-cell wrap controls")
