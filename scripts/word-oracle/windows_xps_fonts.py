"""Original XPS font inputs, excluding proven redundant same-face subsets.

Deobfuscation follows XPS embedded-font GUID order; decoded programs remain
byte-identical. The original XPS is never modified. A strict Unicode subset
is omitted only when font identity, all mapped glyph IDs/metrics/outlines and
shaping table coverage agree, avoiding cross-face loss of space kerning.

https://learn.microsoft.com/en-us/dotnet/api/system.windows.xps.packaging.xpsfont.obfuscatefontdata
"""
from contextlib import ExitStack
from pathlib import Path
from io import BytesIO
from zipfile import ZipFile
import errno,hashlib,json,os,tempfile,uuid
from fontTools.ttLib import TTFont

def signature(face):
    names=face['name'];head=face['head'];os2=face['OS/2'];hhea=face['hhea']
    return (names.getDebugName(1),names.getDebugName(5),names.getDebugName(6),
            os2.usWeightClass,head.macStyle,head.unitsPerEm,head.fontRevision,
            os2.xAvgCharWidth,hhea.ascent,hhea.descent,hhea.lineGap)

def font_covers(complete,subset):
    if signature(complete)!=signature(subset):return False
    full,part=complete.getBestCmap() or {},subset.getBestCmap() or {}
    if not part or not set(part)<set(full) or not set(subset.keys())<=set(complete.keys()):return False
    if 'glyf' not in complete or 'glyf' not in subset:return False
    for tag in ('kern', 'GPOS', 'GSUB', 'GDEF', 'JSTF', 'BASE', 'fpgm', 'prep', 'cvt '):
        if tag in subset and complete.getTableData(tag) != subset.getTableData(tag):
            return False
    seen = set()
    def same_glyph(a, b):
        pair = (a, b)
        if pair in seen:
            return True
        seen.add(pair)
        if complete.getGlyphID(a) != subset.getGlyphID(b) or complete['hmtx'][a] != subset['hmtx'][b]:
            return False
        left, right = complete['glyf'][a], subset['glyf'][b]
        if left.compile(complete['glyf']) != right.compile(subset['glyf']):
            return False
        if left.isComposite():
            return all(same_glyph(x.glyphName, y.glyphName)
                       for x, y in zip(left.components, right.components))
        return True
    if not all(same_glyph(full[code], part[code]) for code in part):
        return False
    return True

_CACHE_LINK_LIMIT = 900


def font_io_path(path):
    # CreateHardLinkW needs the extended spelling even when Python's other
    # file APIs can already open the same long, versioned fixture pathname.
    if os.name != 'nt':
        return path
    absolute = str(path.resolve())
    if len(absolute) < 248 or absolute.startswith('\\\\?\\'):
        return path
    if absolute.startswith('\\\\'):
        return Path('\\\\?\\UNC\\' + absolute[2:])
    return Path('\\\\?\\' + absolute)


def link_cached_font(decoded, digest, cache_directory, destination):
    """Keep exact original bytes while rotating before NTFS's link limit."""
    cache_directory=font_io_path(cache_directory)
    destination=font_io_path(destination)
    if hashlib.sha256(decoded).hexdigest() != digest:
        raise ValueError('Original XPS font cache digest changed')
    for group in range(1024):
        suffix = '' if group == 0 else '.group-' + str(group)
        cached = font_io_path(cache_directory / (digest + suffix + '.ttf'))
        if not cached.exists():
            with tempfile.NamedTemporaryFile(dir=cache_directory, delete=False) as output:
                output.write(decoded)
                temporary = Path(output.name)
            try:
                try:
                    os.link(font_io_path(temporary), font_io_path(cached))
                except FileExistsError:
                    pass
            finally:
                temporary.unlink()
        if cached.read_bytes() != decoded:
            raise ValueError('Original XPS font cache bytes changed')
        if destination.exists():
            if destination.read_bytes() != decoded:
                raise ValueError('Original XPS destination font bytes changed')
            return
        if cached.stat().st_nlink >= _CACHE_LINK_LIMIT:
            continue
        try:
            os.link(font_io_path(cached), font_io_path(destination))
        except FileExistsError:
            if destination.read_bytes() != decoded:
                raise ValueError('Original XPS destination font bytes changed')
        except OSError as error:
            # Another extractor can fill the group after the count check.
            if getattr(error, 'winerror', None) == 1142 or error.errno == errno.EMLINK:
                continue
            raise
        return
    raise OSError('Original XPS font cache link groups exhausted')


def extract_xps_fonts(path,directory,cache_directory=None):
    directory=font_io_path(directory)
    directory.mkdir(parents=True,exist_ok=False)
    if cache_directory is not None:
        cache_directory.mkdir(parents=True,exist_ok=True)
    facts=[];programs=[]
    with ZipFile(font_io_path(path)) as package,ExitStack() as opened:
        for member in package.namelist():
            if not member.lower().endswith(('.odttf','.ttf','.otf')):continue
            encoded=package.read(member);decoded=bytearray(encoded)
            if member.lower().endswith('.odttf'):
                key=uuid.UUID(Path(member).stem).bytes[::-1]
                if len(decoded) < 32: raise ValueError('Truncated XPS font')
                for index in range(32):decoded[index]^=key[index%16]
            if bytes(decoded[:4]) not in (bytes([0,1,0,0]),b'OTTO'):
                raise ValueError('Invalid XPS font signature')
            face=opened.enter_context(TTFont(BytesIO(decoded)))
            tags=[name for name in face.keys() if name!='GlyphOrder']
            if not all(len(name)==4 and all(32<=ord(ch)<=126 for ch in name) for name in tags):
                raise ValueError('Invalid XPS font table tags')
            facts.append(dict(resource=member,original_sha256=hashlib.sha256(encoded).hexdigest(),sha256=hashlib.sha256(decoded).hexdigest(),
                family=face['name'].getDebugName(1),version=face['name'].getDebugName(5),weight=face['OS/2'].usWeightClass,
                fsType=face['OS/2'].fsType,units_per_em=face['head'].unitsPerEm,average_width=face['OS/2'].xAvgCharWidth,
                glyphs=face['maxp'].numGlyphs,unicode_coverage=len(face.getBestCmap() or {}),tables=tags))
            programs.append((decoded,face))
        if not facts: raise ValueError('Original XPS has no font programs')
        for index,(decoded,face) in enumerate(programs):
            fuller=next((j for j,(_,candidate) in enumerate(programs) if j!=index and font_covers(candidate,face)),None)
            if fuller is not None:
                facts[index].update(selected=False,covered_by_sha256=facts[fuller]['sha256'],
                    coverage_proof='Strict Unicode subset, same family/version/style/vertical metrics, same mapped glyph IDs/advances/outlines; all table tags covered')
                continue
            facts[index]['selected']=True
            digest=facts[index]['sha256']
            destination=font_io_path(directory/(digest+'.ttf'))
            if cache_directory is None:
                if not destination.exists():
                    with destination.open('xb') as output:output.write(decoded)
            else:
                link_cached_font(decoded, digest, cache_directory, destination)
    font_io_path(directory/'xps-font-provenance.json').write_bytes((json.dumps(dict(source=str(path),source_sha256=hashlib.sha256(font_io_path(path).read_bytes()).hexdigest(),fonts=facts),indent=2)+chr(10)).encode())
    return facts
