"""Extract short ASCII shaping metrics, never Word coordinate expectations."""
from pathlib import Path
import hashlib, json, itertools, datetime, argparse
import uharfbuzz as hb

ROOT=Path(__file__).resolve().parents[2]
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('font_cache_root',type=Path,help='LOCALAPPDATA root containing the recorded Office cloud fonts')
parser.add_argument('output',type=Path)
args=parser.parse_args()
assert hb.__version__=='0.57.0', 'Reproduction requires uharfbuzz==0.57.0'
read=lambda p:json.loads(p.read_text(encoding='utf-8-sig'))
programs=[dict(p,file=str(args.font_cache_root/p['cache_relative_path']),family=p['legacy_family'],italic='Italic' in ' '.join(p['style'])) for p in read(ROOT/'tests/word-oracle/windows/office-cloud-font-observations.json')['actual_system_full_font_programs']]
widths=[[None]*95 for _ in range(8)]
proportional=[[None]*10 for _ in range(8)]
adjustments={}
identities=[]
for program in programs:
    raw=Path(program['file']).read_bytes()
    assert hashlib.sha256(raw).hexdigest()==program['sha256']
    index=(4 if program['family']==['Aptos Display'] else 0)+(2 if program['weight']>=600 else 0)+int(program['italic'])
    face=hb.Face(raw);assert face.upem==2048
    font=hb.Font(face);font.scale=(2048,2048)
    def shape(text, form, state):
        buffer=hb.Buffer();buffer.add_str(text);buffer.guess_segment_properties()
        hb.shape(font,buffer,{'kern':bool(state&1),'liga':bool(state&2),
                             'pnum':form==1,'tnum':form==2})
        return sum(p.x_advance for p in buffer.glyph_positions)
    for i in range(95):widths[index][i]=shape(chr(i+32),0,0)
    for i in range(10):proportional[index][i]=shape(str(i),1,0)
    for form in range(3):
        singles={chr(c):shape(chr(c),form,0) for c in range(32,127)}
        for state in range(1,4):
            for left,right in itertools.product(singles,repeat=2):
                adjustment=shape(left+right,form,state)-singles[left]-singles[right]
                assert -32768<adjustment<32768
                if adjustment:
                    key=(form,state,ord(left),ord(right))
                    adjustments.setdefault(key,[0]*8)[index]=adjustment
    identities.append(dict(face=index,sha256=program['sha256'],family=program['family'],
                           style=program['style'],version=program['version'],upem=2048))
    print('Extracted actual font design units',index,program['family'],program['style'],flush=True)
assert all(all(v is not None for v in row) for row in widths+proportional)
output=args.output
assert not output.exists()
lines=['// Generated from the eight captured Aptos 2.01 font programs.',
       '// Font design units only; no document coordinates or font bytes.',
       'pub(super) const ASCII: [[u16; 95]; 8] = [']
lines += ['    ['+', '.join(map(str,row))+'],' for row in widths]
lines += ['];','pub(super) const PROPORTIONAL_DIGITS: [[u16; 10]; 8] = [']
lines += ['    ['+', '.join(map(str,row))+'],' for row in proportional]
lines += ['];','// Key: number form (default/proportional/tabular), kern/ligature bits, ASCII pair.',
          'pub(super) const PAIRS: &[(u32, [i16; 8])] = &[']
for (form,state,left,right),values in sorted(adjustments.items()):
    key=(form<<24)|(state<<16)|(left<<8)|right
    lines.append(f'    (0x{key:08x}, ['+', '.join(map(str,values))+']),')
lines += ['];','']
output.write_bytes('\n'.join(lines).replace('pub(super) const', '#[rustfmt::skip]\npub(super) const').encode())
metadata=output.with_suffix('.json')
assert not metadata.exists()
metadata.write_bytes((json.dumps(dict(schema=1,purpose='Font design-unit advances and short ASCII shaping, independent of all document coordinates. The original Word PDF subsets omit OpenType layout tables.',generated_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),uharfbuzz=hb.__version__,harfbuzz=hb.version_string(),programs=sorted(identities,key=lambda r:r['face']),pair_rows=len(adjustments),data_sha256=hashlib.sha256(output.read_bytes()).hexdigest()),indent=2)+'\n').encode())
print('Saved measured font units',len(adjustments),'sparse pair rows',output.stat().st_size,'bytes',flush=True)
