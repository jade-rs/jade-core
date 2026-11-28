"""Generate authored stops near a Windows Word paragraph's right margin."""

from copy import copy
import re
from zipfile import ZIP_DEFLATED, ZipFile

from generate_windows_floating_tab_stop_cases import FIXTURES, MODES
from generate_windows_right_float_tab_cases import PREFIX as RIGHT_FLOAT_PREFIX

PREFIX = 'word-windows-edge-paragraph-far-tab-'
LATER_FLOAT_PREFIX = RIGHT_FLOAT_PREFIX + 'later-float-'
POSITIONS = (9000, 9600, 9700, 9800, 9920, 9940, 9960, 9980, 10020, 10080, 10100, 10200)
VARIANTS = ('single-left-far', 'multi-far-first', 'multi-far-adjacent')
LATER_FLOAT_VARIANTS = ('far-first', 'far-adjacent', 'far-after-two-wide', 'default')
ORIGINAL_STOP = b'w:pos="8400"'
ORIGINAL_HEIGHT = b'w:tblpY="7200"'


def cases():
    return {f'{variant}-{position}' for variant in VARIANTS for position in POSITIONS}


def later_float_variants():
    for variant in LATER_FLOAT_VARIANTS:
        yield variant, 'normal', 8400
        if variant != 'default':
            yield variant, 'long', 10200


def later_float_cases():
    return {f'{variant}-{name}' for variant, name, _ in later_float_variants()}


def set_position(document, position):
    if document.count(ORIGINAL_STOP) != 1:
        raise ValueError('Expected one authored stop in the paragraph template')
    return document.replace(ORIGINAL_STOP, f'w:pos="{position}"'.encode())


def write(source, document, destination):
    with ZipFile(destination, 'w', ZIP_DEFLATED) as target:
        for part in source.infolist():
            contents = document if part.filename == 'word/document.xml' else source.read(part)
            target.writestr(copy(part), contents)


def generate(output=FIXTURES, modes=MODES):
    output.mkdir(parents=True, exist_ok=True)
    for mode in modes:
        for variant in VARIANTS:
            template = FIXTURES / f'{RIGHT_FLOAT_PREFIX}{variant}-gap-100-compat{mode}.docx'
            with ZipFile(template) as source:
                document = source.read('word/document.xml')
                tables = re.findall(rb'<w:tbl>.*?</w:tbl>', document, re.S)
                floating, = (table for table in tables if b'<w:tblpPr ' in table)
                document = document.replace(floating, b'', 1)
                for position in POSITIONS:
                    name = f'{PREFIX}{variant}-{position}-compat{mode}.docx'
                    write(source, set_position(document, position), output / name)
        for variant, label, position in later_float_variants():
            template = FIXTURES / f'{RIGHT_FLOAT_PREFIX}multi-{variant}-gap-100-compat{mode}.docx'
            with ZipFile(template) as source:
                document = source.read('word/document.xml')
                if document.count(ORIGINAL_HEIGHT) != 1:
                    raise ValueError(f'{template.name}: expected one floating table')
                document = document.replace(ORIGINAL_HEIGHT, b'w:tblpY="13200"')
                if label == 'long':
                    document = set_position(document, position)
                name = f'{LATER_FLOAT_PREFIX}{variant}-{label}-compat{mode}.docx'
                write(source, document, output / name)


if __name__ == '__main__':
    generate()
