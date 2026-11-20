"""Reproduce original Times + explicit Japanese compatibility/style controls."""
from io import BytesIO
from zipfile import ZipFile
from lxml import etree as E
from generate_windows_aptos_spacing_probes import ROOT,changed_parts,xml,q
from generate_windows_times_script_slot_probes import generate as script_slots


def generate(root=ROOT):
    for name,data,specification in script_slots(root):
        if not name.startswith('word-windows-times-ea-style-mode-arial-'):
            continue
        with ZipFile(BytesIO(data)) as package:
            document=E.fromstring(package.read('word/document.xml'))
        run,=document.iter(q('r'))
        run.find(q('rPr')).find(q('rFonts')).set(q('eastAsia'),'MS Mincho')
        facts=dict(specification,east_asia_font_family='MS Mincho')
        yield name.replace('-arial-','-msmincho-'),changed_parts(data,{'word/document.xml':xml(document)}),facts
