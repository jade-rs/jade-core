"""Verify immutable, unpromoted Windows Word captures and their source identity."""
import hashlib
import json
from pathlib import Path
import re
import zipfile

from check_platform_coordinates import ROOT, checked_file, read_json, require, repo_file

INDEX = Path('tests/word-oracle/windows/observations/index.json')
INDEX_SHA256 = '158ce28ce843725d14008e544f9be65e56df34b4605a7637148a419c64f49ded'


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
                expected_files = [(document['pdf'], document['pdf_sha256'])]
                pages = document['metafiles']
                require([p['page'] for p in pages] == list(range(1, document['pages'] + 1)),
                        'Original page coverage changed')
                expected_files += [(p['file'], p['sha256']) for p in pages]
                for filename, expected_hash in expected_files:
                    require('/' not in filename and '\\' not in filename, 'Invalid raw evidence path')
                    require(hashlib.sha256(bundle.read(filename)).hexdigest() == expected_hash,
                            'Original Word PDF/EMF changed')
                    members.add(filename)
                count += 1
            require(len(bundle.namelist()) == len(members) and set(bundle.namelist()) == members,
                    'Unexpected/duplicate/missing archive files')
    require(len(identities) == 254 and count == 1023, 'Original observation coverage removed')
    for rejection in index['rejected_sources']:
        checked_file(root, dict(file=rejection['source'], sha256=rejection['source_sha256']))
        require(bool(rejection['word_product_version']) and bool(rejection['reason']),
                'Missing rejected-input provenance')
    return len(identities), count, len(source_hashes)


if __name__ == '__main__':
    captures, documents, unique = verify()
    print(f'Verified {captures} original Word captures / {documents} documents / {unique} source byte groups')
