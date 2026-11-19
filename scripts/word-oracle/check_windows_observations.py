"""Verify immutable, unpromoted Windows Word captures and their source identity."""
import hashlib
import json
from pathlib import Path
import re
import zipfile

from check_platform_coordinates import ROOT, checked_file, read_json, require, repo_file

INDEX = Path('tests/word-oracle/windows/observations/index.json')
INDEX_SHA256 = '68f17dd9eb6a8e62352f784f5c37102e5893ad18a5d55541795446983e59b03e'


def verify_rejected_attempt(root, rejection, attempt):
    """Retain a failed Word open without treating it as a coordinate reference."""
    archive = checked_file(root, dict(file=attempt['archive'], sha256=attempt['archive_sha256']))
    word = attempt['word']
    require(re.fullmatch(r'\d+\.\d+\.\d+\.\d+', word['product_version']),
            'Rejected attempt requires the complete Word version')
    require(re.fullmatch(r'[a-f0-9]{64}', word['executable_sha256']),
            'Rejected attempt requires executable identity')
    require(archive.parent.name == 'word-' + word['product_version'], 'Wrong rejection version')
    require(attempt['open_mode'] == 'normal-read-only', 'Wrong rejected open mode')
    with zipfile.ZipFile(archive) as bundle:
        require(set(bundle.namelist()) == {'capture.json', 'inputs/source.docx', 'capture-error.log'}
                and len(bundle.namelist()) == 3, 'Rejected attempt contains drawing expectations')
        raw = bundle.read('capture.json')
        require(hashlib.sha256(raw).hexdigest() == attempt['capture_sha256'],
                'Rejected Word metadata changed')
        capture = json.loads(raw.decode('utf-8-sig'))
        require(capture['schema'] == 2 and capture['platform'] == 'windows'
                and capture['status'] == 'failed' and capture['documents'] == [],
                'Rejected Word open must not become a successful drawing capture')
        require(capture['word'] == word and capture['capture_id'] == attempt['capture_id'],
                'Rejected Word provenance changed')
        require(hashlib.sha256(bundle.read('inputs/source.docx')).hexdigest()
                == attempt['source_sha256'] == rejection['source_sha256'],
                'Rejected source identity changed')
        error = bundle.read('capture-error.log')
        require(bool(error) and hashlib.sha256(error).hexdigest() == attempt['error_sha256'],
                'Original Word open error changed')


def verify_mac_input_coverage(index, root=ROOT):
    """Every imported Mac source needs Windows evidence or an explicit refusal."""
    observed = {d['source_sha256'] for c in index['captures'] for d in c['documents']}
    rejected = {r['source_sha256'] for r in index['rejected_sources']}
    manifests = sorted((root / 'tests/word-oracle/mac-elsa-synthetic').glob('*/manifest.json'))
    require(bool(manifests), 'Missing imported Mac corpus')
    counts = [0, 0]
    for path in manifests:
        manifest = read_json(path)
        digest = hashlib.sha256(path.with_name('source.docx').read_bytes()).hexdigest()
        require(digest == manifest['sourceSha256'], 'Imported Mac source identity changed')
        if digest in observed:
            counts[0] += 1
        elif digest in rejected:
            counts[1] += 1
        else:
            require(False, f'Mac original missing Windows observation: {path.parent.name}')
    return tuple(counts)


def original_evidence_files(document):
    files = [(document['pdf'], document['pdf_sha256'])]
    files += [(page['file'], page['sha256']) for page in document['metafiles']]
    if 'xps' in document or 'xps_sha256' in document:
        require(isinstance(document.get('xps'), str) and document['xps'].endswith('.xps')
                and isinstance(document.get('xps_sha256'), str)
                and re.fullmatch(r'[a-f0-9]{64}', document['xps_sha256']),
                'Incomplete original Word XPS evidence')
        files.append((document['xps'], document['xps_sha256']))
    return files


def verify(root=ROOT):
    require(hashlib.sha256((root / INDEX).read_bytes()).hexdigest() == INDEX_SHA256,
            'Original Word observation coverage changed')
    index = read_json(root / INDEX)
    for record in index.get('font_facts', []):
        checked_file(root, record)
    require(index['schema'] == 1, 'Unknown observation schema')
    identities, source_hashes, count = set(), set(), 0
    for record in index['captures']:
        archive = checked_file(root, dict(file=record['archive'], sha256=record['archive_sha256']))
        with zipfile.ZipFile(archive) as bundle:
            original = bundle.read('capture.json')
            require(hashlib.sha256(original).hexdigest() == record['capture_sha256'],
                    'Original capture metadata changed')
            capture = json.loads(original.decode('utf-8-sig'))
            require(capture['schema'] == 2 and capture['status'] == 'complete'
                    and capture['platform'] == 'windows', 'Invalid Word observation')
            require(capture['capture_id'] == record['capture_id']
                    and capture['word'] == record['word'], 'Word provenance changed')
            word = capture['word']
            require(re.fullmatch(r'\d+\.\d+\.\d+\.\d+', word['product_version']),
                    'Complete Word product version required')
            require(re.fullmatch(r'[a-f0-9]{64}', word['executable_sha256']),
                    'Word executable identity required')
            identity = word['product_version'], capture['capture_id']
            require(identity not in identities, 'Duplicate observation')
            identities.add(identity)
            require(archive.parent.name == 'word-' + identity[0], 'Wrong Word version directory')
            documents = {d['name']: d for d in capture['documents']}
            require(len(documents) == len(capture['documents']) == len(record['documents']),
                    'Document coverage changed')
            members = {'capture.json'}
            for source in record['documents']:
                document = documents[source['name']]
                require(document['sha256'] == source['source_sha256'], 'Source identity changed')
                relative = 'inputs/' + document['name'] + '.docx'
                data = bundle.read(relative)
                require(hashlib.sha256(data).hexdigest() == document['sha256'],
                        'Original Word input changed')
                require(bool(source['source_aliases']), 'Missing source mapping')
                for alias in source['source_aliases']:
                    require(repo_file(root, alias).read_bytes() == data, 'Repository source differs')
                source_hashes.add(document['sha256'])
                members.add(relative)
                pages = document['metafiles']
                require([p['page'] for p in pages] == list(range(1, document['pages'] + 1)),
                        'Original page coverage changed')
                expected_files = original_evidence_files(document)
                for filename, expected_hash in expected_files:
                    require('/' not in filename and '\\' not in filename, 'Invalid raw evidence path')
                    require(hashlib.sha256(bundle.read(filename)).hexdigest() == expected_hash,
                            'Original Word PDF/EMF changed')
                    members.add(filename)
                count += 1
            require(len(bundle.namelist()) == len(members) and set(bundle.namelist()) == members,
                    'Unexpected/duplicate/missing archive files')
    require(len(identities) == 1783 and count == 3044, 'Original observation coverage removed')
    for rejection in index['rejected_sources']:
        checked_file(root, dict(file=rejection['source'], sha256=rejection['source_sha256']))
        require(bool(rejection['word_product_version']) and bool(rejection['reason']),
                'Missing rejected-input provenance')
        for attempt in rejection.get('attempts', []):
            verify_rejected_attempt(root, rejection, attempt)
    verify_mac_input_coverage(index, root)
    return len(identities), count, len(source_hashes)


if __name__ == '__main__':
    captures, documents, unique = verify()
    print(f'Verified {captures} original Word captures / {documents} documents / {unique} source byte groups')
