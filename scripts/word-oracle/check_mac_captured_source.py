"""Assert the exact captured Mac DOCX on an actual macOS renderer.

The historical malformed original and its normalized Windows input are distinct.
Keep both; compare each Mac render with the unchanged Word-for-Mac capture.
Original Windows Word PDF fonts supply deterministic font programs, not positions.
"""
import argparse
import gzip
import hashlib
import json
import subprocess
import sys
from pathlib import Path
from zipfile import ZipFile
from check_platform_coordinates import (ROOT, archive_reference, checked_file, compare,
                                       extract_fonts, pdf_origins, read_json, require)
from check_windows_font_coordinates import require_original_font_programs_registered

ORIGINAL = 'crates/jade-docx-native-render/tests/fixtures/word-mac-probe-floating-order-malformed.docx'
ORIGINAL_SHA256 = 'eb417a0f7dc283af99425b43a28a356f90ed2ba5a53112e93508b763996b6ba7'
CONTROL = 'public/fixtures/word-mac-probe-floating-order.docx'
CONTROL_SHA256 = '32e7c583bddf915d4a036bc87222064b835993347fc52b0f9eabc8c912c6db96'
REFERENCE = 'tests/word-oracle/mac-probes-20261019/word-mac-probe-floating-order.json'
REFERENCE_SHA256 = 'cb175d41ea402a4d627322a067593deb8a62a9a11b079e75dbb8aded5c045504'

# Immutable imported Word-for-Mac captures. Windows PDFs supply font programs
# only; their coordinates never become Mac expectations. Missing version fields
# in the historical Mac manifests remain missing.
SHARED_ORIGINALS = {
    'courier-new-line-metrics': (
        'a17e0c561725568bcb95b5549cf1fd247af6e34ef34aeba411a38f9b32a08f27',
        'cefeb9d78d82d84607f634f545fcb86c0275a72d3cea4c493ab16ef39cd3b4fe',
        '3da277d5d09f772980616c41dc666a248176ee202cdbb4e01ef47944bfe3f21f', 4, 486),
    'courier-new-regular-adjacent': (
        '89ef3a72d6322925150d96932f407e839cbb1667052966442c76a0407a211465',
        'ec16d63256e3db4cbbd33a8cb982f41d28e6a00d4bbb93ae0a46648182a25032',
        '9e8f3e65417b9425a7d95cdb5e0c0f02f610ca357ea64332c53c34db6961fd14', 4, 390),
    'probe-column-fixed-dxa-4400': (
        '4ea5548ae2e68907aba9b8dc43e834ec07f293f0bb0c04de70a81fe567085b23',
        '96702526e1b4a5eba635d86473921d23c86fcbc87c1621b81029c16a296f54f3',
        'e59ebb510ceefa817f313a79cde0ffd50b5e6af9320165404d3b1bc12ac20deb', 2, 107),
    'probe-column-fixed-dxa-6800': (
        'be054cef36e47288907da7f29e4c135bf4bcd40adec5ddb6432790cbc4b0a861',
        '33193bf919586aba58541ceb9462e48c5f46c5e497dc32c1ae918f02d6e36ee5',
        '3e98f1772071be15003acb84ff297aac762245ab8e55d9f395748d41f6bb3c4e', 2, 107),
    'probe-width-single-dxa-2400-dxa-2000': (
        'a611a63890935fcc6a2a831fc50c19b2641846a8f7fda4a286ba4c033860a358',
        '265bb130473d95a71125d824cc15069f6276d02283d8af1be770cecca45b5475',
        'c150979819c2f84b61e629b95435559e8170b2d9920d757bc2aa97d41ecd8cb9', 3, 107),
    'probe-continuous-column-explicit-break': (
        '036d1a09b7eca1f005fd00fa800c6342951c0ff9fa32574606719d67f2f8c648',
        'ab464ad7e5e0893d4cd29a0ad55d0e083cf5cbd933d674d7fed76fe7fff66012',
        '1c8f463d72dcafad2635cca5c14e8575da8447ea1d8fb0edc4ed2006d6db576d', 1, 242),
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-1800-twips-start-column-1': (
        'b0656953e16a6b5ebe488dfcc588e1f84f1ea1c0cea116832a9e9b7a2e3cbd06',
        '9b2573c47db38b50d404ddb5316127eda1303c759f2b9df2653bcd811afd3945',
        '9912015a19baf8ce41cc72029c85928c47628ce86fcdb17e135546675d543f7b', 8, 1014),
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-1800-twips-start-column-2': (
        'eca02091b6b3faa96fd2f247dbbb4d3f5b8b609c9871d2a5f01b31c2223fcf0c',
        'f77d146d0cb47096eca983cbf22cbb907a4fa681f6d6936248e554e680053918',
        'b706f77852872b3296247a1ad5053d8aeff048484cddd82c380584fe3ed438d6', 8, 1011),
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-1800-twips-start-column-3': (
        '08637e628e39e2b1172d45327d0b1275fa2ea8fb67316be49a4520a0a1e9f401',
        '1bdb9fdb483ee80dc038eb6fa9bfb1a466e07afa2a8f25527ee74ca87352342e',
        'eaa65938597042ea3949ad276fd01fbccde3043d56d17307b252c128d88b80e8', 8, 1011),
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-2160-twips-start-column-1': (
        '53d1c76343ca732fb20e04d0c3a03086b9e8f7e2865c9a9fbd983c94eb546b7f',
        'b0a86a767ebf25ec7ddb41964a73599b8b1bced5b4f813d9df0c02b0715d9f72',
        '7588e47a75fb0e63a40738744694ae650630e25df66e594e055cd8a5dcf8cfb6', 9, 1023),
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-2160-twips-start-column-2': (
        '1c9e71b0420b4f2b250a7343f62e129fceca8e9021ff6afc31936f69e107a2ba',
        'eaa6c983da8ddceabddb6ceee0c249b354205ecdb1512241af923294299efdde',
        '48b68add3efca03ad611ee276bb42b8561186a4699a060d16a1ba4acdb32a400', 9, 1020),
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-2160-twips-start-column-3': (
        '6e8247c12a218ddf0e56c0cd7db669a7aa3e5d17cf96ffb1bedf2e58fe25c11e',
        'f05ca5d9e001eed87ec444b82d7555544d232f1f6a581ad499a0250ede25c3f5',
        'dc63ceca066d4ee001b00f659561763196d00545bd378e111b196283e86b0ca9', 9, 1020),
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-2460-twips-start-column-1': (
        'cab1e89ef0cb109481dcac0f75f177215c3328eb2abb20b1fe20f86ee04d7791',
        'b2daaef79e146772f56e363dd2535bba6f44536f0e87639393d453a06636e391',
        'ed6b0f6043f2648aca44e28916467e11e4ef8854b992a6274ad0855328c2b395', 10, 1035),
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-2460-twips-start-column-2': (
        '451e0512b957693bb77e6b529a5f915b4da5b048a15ad8c867a3f74d5d5d3b28',
        '472c82e406d96c64f7e398e855184bca80a782d7e1ed5e807cb39eb1c4e67eaf',
        'fb5600a150dc90413f2639c3c98cf799639d964f749b914c33bb64c21c02847d', 10, 1032),
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-2460-twips-start-column-3': (
        'f2e3d8e8dc3045b45ca7d6c626554c769f8faf1a382c51828f139450d80b21ec',
        '21ec0c533aeab78fd8ce355824217ea687da5dd136d071983137346a74478743',
        '22652b26374b18c269457e992f05a63ae653671dc9dca10fb1441cc176ec9f98', 10, 1029),
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-start-column-1': (
        'db45dbc9ca43ba222e6e56da73a368603ed4960875b025204c04fe054409132a',
        '902578ce2fb7212f75e4643d64fc34b4db4115f57f722b4eb87e337114d4276d',
        '8df64b90a35328cad5fa891e708736d040d10b2d589dd2d75e60962fccfb3feb', 6, 1002),
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-start-column-2': (
        '94136fa4c50691a4266f40bf15d84d080999210c8fb18e1bb91625b2b49dc12c',
        '25597147aac3e15215bd9f63849c17902bd8402654c0c842d3c2d561b81554b8',
        '7d701b6c02574184ee241f760beba96eff8ff4655c4332caeee67948b052f149', 6, 999),
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-start-column-3': (
        '31b20a177813df009a6b1745ad8cfd425781e5e7652939b059c77e3e892909ae',
        '81e5aac0c19e661dd056453c5c7bb3170918e12f1cc65d7a3c6af14ca0d63d0f',
        'c96ce48e94adcac630300bdb159517c203f4b60390b151a0d7dc2e5bc9bc6522', 7, 999),
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-tall-header-start-column-1': (
        '16f73bae7e7e6cfedcc9913c9025a85d507b894e6bb35521086fc769339c0c6a',
        '1bbe47750aa9f0c54f6fece685fa1de3885a9fca08a372aef263e880185e02c4',
        '047c9cae4ab889ee0c18818a4e53752f067e75bd58562ec7fc85731a298c2149', 12, 1047),
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-tall-header-start-column-2': (
        '3cbd2ac87bf90fed62071dd17462ae30888b6737f34440a8ae5d9d129734db67',
        'c066fe63f8a9eae9e9d3b554e53ba622aa374df5242e49bcbbbd173f984a9647',
        'f895721387dbfd0d5d49ea81b1f2f101fe161aa42344ce729e0c8cd2875b143b', 12, 1047),
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-tall-header-start-column-3': (
        '6886271736c086c2510908223f58f967c9c4579f126bd2cb7e039c94131385b8',
        '1dcfb94d52ce40b73e164aeee12e9376ca173e6ab046fef1ee9e53df0a51b911',
        '03e9c64dd5aea1add4addf76eddbe90a34a719e8a7a494111ed41d851063d209', 12, 1044),
    'probe-fixed-auto-table-grid-2000-cell-pct-1250': (
        'be95c64a1a69e75986bf913c8f5dab2875b8522024b7c6be69b0bc09f392eb5d',
        'fcfede5701e593c8c0e25df865fcb3b6c310262ec2f3256061e8d816b96f54de',
        '38c12e496292da1d999e69ae621e127d9fe70d5f4a203b24bff8576a38927a9b', 3, 107),
    'probe-fixed-auto-table-grid-1000-cell-pct-2500': (
        '2aa4101101d3b6fd8ab7c99736cebcda3278088e786b8e916574ca790b28add5',
        '17c2be32321b8ef24c4fbe9401c0f4ca027de239efe5ec4d0788d0ea3b545674',
        'd6606df22b93edfd37faedda873a7b175152228e2e715aea49cc8a29e16cbbd6', 3, 59),
    'probe-fixed-auto-table-grid-2000-cell-pct-5000': (
        '5bc7d88f7f89d23ce3e4bd986c38ae1a34c29b8c7e2fd16c953f947908e72767',
        '3d661bacb46f4f70792afdedaa2ae34fc7dc9d69aebdb61e3844c1708a8b2a12',
        'b404095f8e702faf86225ff49e25ca7627b0d74e1af24479d26df6ffc0ed5d3e', 3, 56),
    'probe-fixed-auto-table-grid-4000-cell-pct-2500': (
        '4b7ee04b8912d2e03b26013309a7a2a96de50ff8ed52bc4748587473b1047c2b',
        '0b68b4021f26f75ebafd0da04ae7252e6d91f771f35c50ccc6eabe8b15addc62',
        'd6606df22b93edfd37faedda873a7b175152228e2e715aea49cc8a29e16cbbd6', 3, 59),
    'probe-unequal-variant-narrow-first-2-9-dxa': (
        'c1ff95bbbba66d57c63e1deaac29a8d5eee89c09d5c684ef49285b93d33c7e97',
        'd19f7e75200d0c7d835dc9c41b05a1eacddc2b14ce3a9505fc7d361bdc365330',
        '8566c1efacd37b11c5ae71e5ad3bc36a9e6cc1b40a4aa9518140a503bebc7b99', 2, 96),
    'probe-width-equal-auto-0-pct-2500': (
        'b5b8e4255b28f3848752fec0e32a8a51ea104449ef3b234c5d3490f1d0823fdf',
        'cab1903237094d01f3e164a0692876239d94df78ab4b524385cdb14447514927',
        '9db8f29647e323c8d8bdded50b9103d64097013356ff46473c99db26a4cd3b3b', 2, 59),
    'probe-width-single-auto-0-pct-2500': (
        '507021382dc70224b5842454c13b3a28c4685824e65feb02bbf5257c7c52cfea',
        '919a01910562695afa8d76c491ea0364dcc2f116743c42f27b67f84529f08d97',
        'd6606df22b93edfd37faedda873a7b175152228e2e715aea49cc8a29e16cbbd6', 3, 59),
    'probe-unequal-autofit-wrapped-header': (
        'bfd024d72540b3dcc0be838e37713438e17502e9b83e70c990efadafb96ee597',
        '84641c8a20c2ab6b87dcb2863d7c3083716268c0e8bf36f2200587c81dd042ff',
        'e257cf3ee6c0c3c99601b3cb99244cd32a9854d91e29d8dbf3844662924a7101', 8, 1709),
    'probe-unequal-center-narrow-first-2-9': (
        '095b0e6682e3dadcd39d03a610e1fd23c0a384574db01145882a1266c704c561',
        'f49db3f04a8c32081ad0cfc3b553ce7f36037bdc07e31183ef2b39a2471abf71',
        'd6bdef4bc1aaf164d25ad63d5eedd19a503fcb9e0c4bca70d180a59a311090f5', 2, 152),
    'probe-unequal-center-three-10': (
        '357abec2fde6a5121d936be7e9cdc07e2aeb5b0846ccf358a51d766c97fb38c1',
        '3bbb637c143cc3cd9678e0c4b3b54ff762f07b495a4e3fc8559a17202072a6d5',
        '8b3d72dbd72fcdebf464605a95d75798d7e5237dc529dbd082efc1cf4cb374c0', 1, 115),
    'probe-table-flow-unequal-start-column-1': (
        '13af114981aee89594d7eb61aa92ba6bb3bede575f23d6ee67eed43365479217',
        'c8a7210681afcbe6871912375601eaa0e598bd37db57f60f71d620bd03fc745b',
        'f21f8e8374c8786be706b4bd4e30fa40d622db5436ed8d848ab7ea560e2e3b20', 2, 215),
    'probe-table-flow-unequal-start-column-2': (
        'dcd2f95e62bc2682f50bc840c0a39f06f557af0c465275a69316181ea614c8f3',
        'c1499607d0351c8155a2f7d20aa71c7f796f72b09e4278c51f3cc8c1b5503eef',
        '78a382c3f29283594a71304d78da086a1143db1351eb97339950714f33d3bcb7', 2, 215),
    'probe-unequal-autofit-section-width-matrix': (
        '08cd51d3d5d3d2d4f7f2ba8522107cff36cc262611bb9f542bc8e8afb56ef709',
        '8368a442e58884c2dcf2db7e111b7a351ae3678f9b7a5667876514b274a94538',
        '8b3e07ccd875f24e482c84e979b4510b7f388ccfda17f560d399699d68b8a5a4', 8, 1421),
    'probe-unequal-center-narrow-first-1-1': (
        '5fc46463e9ff40f8475f8dfdc57e91699e7c05d7cf88090f77040744bbf356fc',
        'baef2dc9d6a36105456bc2f857b47bc56d2e381c026f9cebdb81893280b4886a',
        '93085c826be5e440c82f7c2f892c5d3879795fbf58071be8282e20f653dbcd42', 1, 29),
    'probe-unequal-center-narrow-first-1-9': (
        'afa13d09239051235b4115f4cfcd6f9e5cbf261693ea928d2f1f01439addbc59',
        '87697f58f44e03182dabd803a59679edea6136e5deab9bee2681dbeef5acdf43',
        '4cb0104d7c2d48324b994cb416ff3a076edf293ac1f629352fe1f897ec428ef6', 1, 152),
    'probe-unequal-center-three-13': (
        '311e230e5d642ef7fa16e132bda6c61c1eb47f64de479e424d12d9cde30e61c0',
        'd98cbe127c70fdd75c2f81f8c98a8598e2845a518bd97ff5726ffb9f985f2c6d',
        '92c3a47dfb00ccf6ee0e935c95cbf995eb5b3b1056be63ed0309ad9ce03cfef1', 1, 139),
    'probe-unequal-center-three-17': (
        'ece700f309801cba9b6d39caae43bb726950d26ff9da71cc48921363b3c52452',
        '823b4892f776fd191fb2b242039087187e6ae8e46ab75388c0f6a6f75e91ba04',
        'c33a67c0747bcb5efcf8421f80f17bd26b07f1b27e992b95566012bc66fa2ac2', 2, 174),
    'probe-unequal-center-three-24': (
        'c6bc75fd9da0f7ee564dccb7ebf5d28ceb469af811df8cf8033e6c5aaff12205',
        'b8d26db314e953beb0848bb915db89bff980f51421abf03ffba5dd8b352c2f92',
        '58ab1e66028ee8872f7725764fb6a51b78afa80c1af08f1971ce435738793c7e', 2, 233),
    'probe-unequal-center-three-4': (
        'c7434e68f2537eeba58ad2b7d8915688fa5f3653900374d43d496728a9c6ecea',
        '251cffbfaf59cffcc11be4d44e8ee0b5c8f9080bfa0024c7d08f895467e8f0e5',
        'b19d835796a021ad172c4c27abbb5b8c79c9b82929e0094fc3da33155516892b', 1, 61),
    'probe-unequal-center-wide-first-1-1': (
        'bf865c8f8f8271a8713c73c04331fbdf8517c1e206b3d20646883e09d018f0ef',
        'e0dc2a7af1b28040625dfef8c6246df3054008ca87273b7e5aa7bc2314f5a450',
        '68d70940ce77b109896f7085b8b811bcad97dfc0a2d91f04b8d98ae55d2a3036', 1, 29),
    'probe-unequal-center-wide-first-1-9': (
        '08cecf1bdab79df3c8a947915d4e7202bd10c773c64167200f602cc36edec52c',
        '2e350e90442271e2028a9ba33ff0635570a6d7ec18367a7ed58eb1982bf39ff3',
        '15d22d406d34926b061178f4e5b813e221f183f3d60503013ada8aef7cf8c890', 1, 152),
    'probe-unequal-center-wide-first-2-9': (
        '0698b4252638b6b45af36a4219ca35e0538cce18ccf6d30b6a7b4ccb1ab20661',
        'a27edd9bce0f6758069acab2902d5fad9719c681afc1be5f9d67fb2a4650dbe7',
        '32152ff40f143bb9ade343c9e1a4f6536dbb18db19c1fdba1cf8f9890103cd79', 2, 152),
    'probe-unequal-control-1-16-with-tail': (
        'c17c196ed38032bd6541eb629d78c10a5d155a0db9b9868292e27920c9a58541',
        'b1f857e65c4081d21a23a86e07c5c3ad15796836c5c417c08dd2ad17e81c4388',
        'b562bee58959f32fe6636162a2252d764b324428bf2759fe4862544b9d722ef2', 2, 158),
    'probe-unequal-control-1-16-without-tail': (
        '7676ca63038fdf22e4144f00608c44ee9cd65672b732f208844a21ba2dc11620',
        '4da4956c2dd2355202dedf30251da2334fadcc6be88ac0a532cafb8668f22bed',
        'd7e9bbbdfcc7e2e5b928bbda84ccf2e201096c63de9a38c8463b2272bd801af6', 2, 145),
    'probe-unequal-control-1-21-with-tail': (
        'f36ef63f7ab2181051775ac7aecaa125f1551bf5f7de6f8c68efdbdc2e1adc8b',
        '8c717b226abe083d3169bd47fda87c53ad5f051dd89dc0f40b70f0d0de0902cf',
        'a2baceb268694f906a9caba545eff6020617784f7d0a76f4e2114d74b97db9a6', 2, 201),
    'probe-unequal-control-1-21-without-tail': (
        'b2fc0a8b484d705c90d43291c035c28b71b7ba3747ac86aa5fb5a18c084497fe',
        '5e43f64742fd7efbe40a48ecffc5810f8f5335434dbff97092f57198d11a7de0',
        '8bb6eae3351c694e8051b9483a8542058b113ace5c2c64a349538ebccf8db149', 2, 188),
    'probe-unequal-control-1-22-with-tail': (
        '175e113a7c448bb7c8af1ba61460e630d3cc0654f5b4b126e2acaadf4041b49e',
        '991f38aa074a5fdf159d9d56bd474d2c052eba0ae29733a8f8dbb2a511f83c59',
        '6ab4747af90c91c9ebd376a33de85f881164fcc9e7175f8a252787cbc6c14650', 2, 209),
    'probe-unequal-control-1-22-without-tail': (
        'd2fb25eef45583414569304e1f79b051bffaf02af632dcce60086aaff06b9581',
        '1e65cb6694af68a20e4f7c3a3943edab671fbf4214012371a7efe704b9915bef',
        'f919a240b46a01043714656c9cb568f7fd0c1845b79773e5227e3d03209c3cfa', 2, 196),
    'probe-unequal-control-1-23-with-tail': (
        '010aedb3550200abf1418e5e07f919390b19e2bfa1fbfa087dbd7035980b042a',
        '45ead4715b5c7f52b4368475debdc78110401e804112a5dd296cf649503e55d5',
        '327bb735abb3184aebfa92ae455c2279278450814ef8bf3adadf2094c63bdacf', 2, 217),
    'probe-unequal-control-1-23-without-tail': (
        '04dfa2c72ef43b7309da526336bd000221ee0b7008021d3e31d879d1c60dac69',
        '058db9d9301800d41226f2b4f24acfde009d275c53573e152663e8b1a76cd71d',
        '7af32974623ffe71d9ae7d1c0e36d483f57594cef53cd9733a6b793d9695490b', 2, 204),
    'probe-unequal-control-1-24-without-tail': (
        '2f07fc8fe7688af193f4f3b98e23c54834a8c5b2e1dd05d7a911b76c21cec013',
        'a9147ef79e2220b44d46426c19062f52fe5e53e0346e412eeac042635c6eb1f0',
        '0c9ce19dcdf4dd429646e450453f9caecedbf910ecd1fa3a1b601a45b8e91954', 2, 212),
    'probe-unequal-control-2-12-with-tail': (
        '839d9da17ebac839d4d137a40555475ceb65c5d3a34e052e833ae5998ce9af96',
        'cfa3b429e9f67b11d2a2f288097f202f4af618fdbfc9629a612ed1b67e970bc3',
        'ebb4163f6e96c05cf38b44d5ec128a7fb368574d33500781a94b5ab197fe5235', 2, 123),
    'probe-unequal-control-2-12-without-tail': (
        '5c4210e4a06ef06475d6e1d36110f1e6bcb1457d29ebced5eebe8ccc9c2d7c92',
        'd56d8ac0dea9c0fd353a55a24a6c87f339d4e25111ec80c363b98d389ba2b45e',
        '9e7b7702d90f6fa3375bdc022cef199c39e624faedcb3c33a80910444aef5d13', 2, 110),
    'probe-unequal-control-2-19-with-tail': (
        'a7fce0b75ca5e1226145eb96ae73b70d0068bb163fdc66a7156444a470bdf645',
        'ddc52dfdea01fa52d53146dbb851cd328e71fdba8bd5423c1d77fa0d7531d861',
        '16ae5c8a4bda9ad24f733db5d79a02594599d13463686836adeae560abb58d18', 2, 182),
    'probe-unequal-control-2-19-without-tail': (
        '6e2e82a70b14fa8f3d5854943c4084aa0c488dac898022229a3862d9f3fb4b69',
        'e001fb77ad20ef37431c0243b86540a4870e8f67d5943938239c1c80e94208c3',
        'ff6bdbad3c8160e097b66c9b1053695e92d87f1af67f156164a6e4c4276e8639', 2, 169),
    'probe-unequal-control-3-14-with-tail': (
        '21cc37dc7f8617d1c4916704eb3bea1080c543299b3c19a42375baaf0465659c',
        'cca14924adf8348817ee8015986c8c480333549b81ce6f8f913ecb173608dc4b',
        '918d764bd35003a517ba8bc652d90ee22bb640562e932f5c9acd28ede9fb84e1', 2, 139),
    'probe-unequal-control-3-14-without-tail': (
        '3a0a56dbcc792dcd9e8442b9634e8da7250cc419e1a4cbe38ad2be506a72195f',
        '220fa40257086740c73b118dfd92e1ad1bb3f253ce82a07cf54bbb9cc2d040ad',
        'fc3e5bce7f22842aa246c08e59432dd4986db23baf4f3a78ca77608c2fa3aca6', 2, 126),
    'probe-unequal-control-3-8-with-tail': (
        'd2dbab22ce5ccc53d484a88539c17e558e1f68aa020672eaf4cca85136acb950',
        '7d60b39dd4c0b451ffb4bdf42c43b2bae717086049f613b144f497bc5d8d8e9c',
        '5cee95e03eb8f81816b13fae6da2b7e5ab38171ebb496f5b0f67b91fa7471b19', 2, 88),
    'probe-unequal-control-3-8-without-tail': (
        '11bb0953c53a01097b38c97e9210bc22f639f6ad14eafd2e9987742b64b8173e',
        'e7ae6b2c6e65854c9d39fb0463d5a692a39ac205f9eaf68884e423943ab19e0b',
        '6799b3d398b525547512bfd75f1af736b6e47a991e60f7f3f727a1e41c4c6473', 2, 75),
    'probe-unequal-explicit-paragraph-width-3300': (
        '961e14f87efe0d33b5c2ccc39bd7d9e4d5e21b34efe8bff7a6610ecb2fa7ce74',
        'f9e8ef9863ad6988d23198f127fe0fbb852d01dc2e15f17c6c0e49fb0a5ea499',
        '9d8acac12a5178e4b9e39f381d3018b364257d69104e021311ffccb506cb0fa6', 1, 62),
    'probe-unequal-explicit-paragraph-width-3566': (
        'aa87e8f7758f314a169799f14bd47a137d664f50dae284cbaa9123178c3c5274',
        '57f53ef3ae4f34a481fd017b8e609b580f440840f4ae18f73e061051a6e99431',
        'd67243aa3ba7233f6cf4544d49e14854f68f9e718c801873c16afd04d06eb245', 1, 62),
    'probe-unequal-explicit-paragraph-width-3800': (
        '4e060b7d222ff620d86bb59670f8c3cb2ccc3dd4f33284793695d5b4f1226b02',
        '8a220347db98df7fc45883d72036522be183668d62817e17aa4a673ecb6f3042',
        'f2bb4cddbf33e287dc71435bdb7ca00282a4f6cb4eef560b3cab859287c0449f', 1, 62),
    'probe-unequal-variant-narrow-first-2-9-center': (
        '21c4550f102a4436e617816765056c3481b7e3041fe1e70b96a173d79ce30057',
        '52df6be32d06c7766642cea3df4e0adfd71031cfbc042cbdbb603aabfac33482',
        'd6bdef4bc1aaf164d25ad63d5eedd19a503fcb9e0c4bca70d180a59a311090f5', 2, 152),
    'probe-unequal-variant-narrow-first-2-9-long': (
        '51118d853ba31aabcbdd9ea844f19f48ca1d4729e9912f87ca3b9310cfd93efd',
        '0358d046e478197cc0f9ebc7637b1d2282a8594b5e9d0e8be6b67ad869a336d4',
        'dab3f836851d490a0bbce9c1bcde8767d0cbf761dbd45328df6035ab02d454c0', 8, 1025),
    'probe-unequal-variant-three-24-center': (
        'ebd4aa3648e5ddc16b76806fcd37bf82dac0e631d05fae248a41d97638a439cb',
        '38ba8e36881a3f1b7f5219876f650f1feb1a521505f57b514d634ff99dd7f90f',
        '58ab1e66028ee8872f7725764fb6a51b78afa80c1af08f1971ce435738793c7e', 2, 233),
    'probe-unequal-variant-three-24-long': (
        '4bd2417e5c618aebc760ea61ebf34a588021a75e51439e982ff8fdf4dbc49d44',
        '35b7f4dec2da657d7f09e326db22f71513c27b0621a6c941eaf5a28ad3eb6815',
        '997b3d478001bfc58057962b6fd3aab640e569601b017d4cc0f7d17b68469aea', 5, 577),
    'probe-width-short-narrow-first-1-1': (
        'b7c18123daf114763d4450a240d72e19fc7f73c41b146014cacf50fb1d604742',
        'f9d7b6af5142eecbeb7083ffcf6589d395167dc9490bac52af57740d7087a5e0',
        '93085c826be5e440c82f7c2f892c5d3879795fbf58071be8282e20f653dbcd42', 1, 29),
    'probe-width-short-narrow-first-1-4': (
        'fb6c2d14084b55d52090d25d322733786500fb01b2e350f173270f45e7938bc7',
        'd558d125a3e00ae59f48c79f37256cdaf0c6cf1920295aa8eb259b9043f0f011',
        '3f309a32330180bcbb3b2ff230aba94c2e4b0e4d5c9f79fc26ad59612394614a', 1, 74),
    'probe-width-short-narrow-first-1-5': (
        '4e30dbb82c14b4c32398b35b7c4eb62c5ac68c1ec1b92750b4895fe75f4821f1',
        'd76aa0a6ebf7c1b3355828f000b2c67c471d304b27fcc6771b4aebf02dd4b768',
        '07465f528a2413be089019e761f4080724bd63ac219dbc270a65b2257c071237', 1, 89),
    'probe-width-short-narrow-first-1-9': (
        '7d70077956b4b248607403dc39acb9e6f63a38b665659c1b4ba3671380a6690e',
        'fe197d8625b04a17229598eb546bffb24fcab4ba94b3afb341b91457f16e8537',
        '4cb0104d7c2d48324b994cb416ff3a076edf293ac1f629352fe1f897ec428ef6', 1, 152),
    'probe-width-short-narrow-first-2-1': (
        'f5775f2cf46b45f3e7e2e43bc97adaac48ec519569aeb58935ce76d5675246f4',
        'f46a4abd1672f664caae15b446ce72a78469a92dfdbe69f4382b0ee6d1298026',
        'fe298f23d526467f0fcb91d4d3cb2f6c1ed0934d5b557e494a72c9ae15a694fc', 1, 29),
    'probe-width-short-narrow-first-2-4': (
        'cbcd0aa5d89b2c1da0051b37f0c094ca8aeca86d247180e21e7adc075b884841',
        '366da2730acaf0bbed254bb6dc35acadbc4c29211fb725a2e410b21b478aae5b',
        '0970d6929048092e4a0346882bba34b918eafd4ecdb6a9a24de4177b38bd6500', 1, 74),
    'probe-width-short-narrow-first-2-5': (
        '50f4200f4a537efd0899fa7c0682eb6d7b140f467816cc054c6beda4e697fc0b',
        'ec2ab8d1a73124f88aafe4d0c685b136508361776bdb5c7b3bde08d93049ce85',
        'ae2d9970e3e03e59c2b11583e9e16b9f9aa3c06226c40c0b0fcf219a404503b7', 1, 89),
    'probe-width-short-narrow-first-2-9': (
        'a40a061d66aaa08808bcbfd50a40fdeb443a382b21060f726609a8ad693b644b',
        'bc02deb1ccdd8fc051a450886dfb72c76b3c656edcae037f62e2bf6f5c0ef55b',
        'd6bdef4bc1aaf164d25ad63d5eedd19a503fcb9e0c4bca70d180a59a311090f5', 2, 152),
    'probe-width-short-wide-first-1-1': (
        '25423b3f1a8b4e7d7e9d4b273a52ab93819bd82a42ed9b9bf8f174ec48cc527a',
        '75dc97c8fe5b2aef3a6add3b0ea87873a8ca5e8c2e5756fdb53c8098c665ed8e',
        '68d70940ce77b109896f7085b8b811bcad97dfc0a2d91f04b8d98ae55d2a3036', 1, 29),
    'probe-width-short-wide-first-1-4': (
        'fd31c25683c274f34310f289d443786f8e6a70c47844162c48ef92bd00abbd00',
        '1e4066c5ae79dc604890991b06f6326ce94197b30bb0992bd49048f06fd313de',
        '3c39c72df244c2bc505075a6ab741df707175edd771453e08aa4e22a3a7565d9', 1, 74),
    'probe-width-short-wide-first-1-5': (
        'e93f8d61e3b3e2280119636d028418eface0193f632e90515dfef8f7ea839008',
        'db0cbe037fa6d80e1d57857956b0828dd0a6f9bafd7a5b3b6a31819a7eae4e30',
        '9826637b6473fd0365b3df7bb9998fce0e043e1cb0594be7d1d5ae210f862783', 1, 89),
    'probe-width-short-wide-first-1-9': (
        '7b65c91b9cabbf8230ea33bc9595dac34e1b130ff8f9d10e2c03277ef86f7100',
        'a8678ec580f6c99138bad9ffb40e30fd87f2128c79575f6ae456227e65f35808',
        '15d22d406d34926b061178f4e5b813e221f183f3d60503013ada8aef7cf8c890', 1, 152),
    'probe-width-short-wide-first-2-1': (
        '386e8d2c41ab93cd74d7fb627cc5b6982f76d4162e22265a892785a63cdab287',
        '1410f05e3838b423b194be40f693b5b10d722f850e2d28bcbb4c65adfb02ed98',
        '85fc5d2bdca2ceb924a447440b7ace8884fbbcce68ebbf1948ffe96ec9aafd01', 1, 29),
    'probe-width-short-wide-first-2-4': (
        'd7c54851298e778cf09a41692a7812d80f0b4f8f151eefb31ec6b98d3575c0cd',
        'c8aa311b350e6a5763a40865dc384b4755eca25bd2f4e7e7c93c363aee2cf7ea',
        '8eec272364fc3c950a8860b790a2c961cd65ec83797253b8cd29a0bf9957c0a2', 1, 74),
    'probe-width-short-wide-first-2-5': (
        '7390527ea510471c334ba49efc71096cf68f9eb1c075224ed51293f89b61b809',
        '47931ac3ab421f09732be7f75e93e50cf78d89f9730a7b32b76b8aba4dc5dfb0',
        '827ba1bfc013877ca333dfd7f88bc8e7abba2c6814f8820df14f3dfa34a72100', 1, 89),
    'probe-width-short-wide-first-2-9': (
        'cb5711ec2fcfd1a903a6e2381f9fb0da55f94f79ca9969d11ef60b63914bbe45',
        'b29d1abb7e6cd4d872e840e8eba9573d7a8a44dae33840daad2905c87c81cadf',
        '32152ff40f143bb9ade343c9e1a4f6536dbb18db19c1fdba1cf8f9890103cd79', 2, 152),
    'probe-width-unequal-auto-0-pct-2500': (
        'f45ba7e0738167c5728cc0a0ce9d1c6c85635a1cc4bc49c931f2db6d354ec359',
        '0457db816025b7c2967d911e3249f7f33562fe0234d936d1fcddac1aad344cb0',
        '18bf27a7d57594491facc5c144aa1ac95cf66a552515dd65732106573bcf3e36', 2, 347),
    'probe-width-unequal-pct-2500-pct-2500': (
        '7c4c0d95bab7a00490477cb885d165cb5045f7ba0740b572df6cdea144c606e7',
        'd90218e7c424f0d693f0b75b5436e0af6d62be6694403979312339456a521f98',
        'aa0d326262cbc9dac0766328236e894ff08ed0ee1569efc884f2fe2abf7351e4', 2, 215),
    'probe-width-unequal-pct-2500-pct-5000': (
        '06e747ca73e8e75f3751cf6d6bd52f73febae928deece315d509e2b098a232a3',
        '5f3e50ae95c03b97c8022f2ae18337e14834ea4b28fe8524ebaf6843db05d18a',
        'aa0d326262cbc9dac0766328236e894ff08ed0ee1569efc884f2fe2abf7351e4', 2, 215),
    'probe-width-unequal-pct-5000-pct-2500': (
        'fc087a2a80e5a758a6de90bfff3a16af0e29474d56edb2eb4b0dd09f18e77d6d',
        '05b21cf82d5a4093a7157eb1e81794c9771c02da8b998f1093be0c65e653db4c',
        '38452ad3a1c6df1d5521872c1a7928abb294507b001284a01fe41d933beedee6', 2, 347),
    'probe-unequal-variant-three-24-left': (
        '653145c06db76e4a3ec26f45fce695783529c9d6335efbf4b1e88420d0d6ee92',
        '9cba4a39a470891b5274d0de11db31788d8d4e763aa0adcd9529c43f89851be4',
        '313a1ad6e8cd802ce5f2446eb756912be29d5dd0430d48980571377f93ae695c', 2, 233),
    'probe-unequal-variant-three-24-right': (
        '6feaf145dd75bbac33dc1193c1d5466513b670b29828392fadbf489f8bfacc28',
        'bc8d3f6a40b2632f24726b80c7d10d92432c5c63fa3a7525dfda205022026273',
        '88c6685aa8c4cd1db1421de9ebbd2e5a3b5add79f01ec7b8e5970c09cfadcc39', 2, 233),
}


def shared_originals(root=ROOT):
    registry = read_json(root/'tests/word-oracle/platform-coordinates.json')
    cases_by_name = {}
    for case in registry['fixtures']:
        cases_by_name.setdefault(case['mac_case'], []).append(case)
    # Reject missing/duplicate required cases before reading any original files.
    for name in SHARED_ORIGINALS:
        require(len(cases_by_name.get(name, ())) == 1,
                name + ': missing or duplicate shared Mac original')
    results = []
    for name, (source_sha, manifest_sha, coordinates_sha, pages, glyphs) in SHARED_ORIGINALS.items():
        case = cases_by_name[name][0]
        require(source_sha != CONTROL_SHA256,
                name + ': normalized control must remain separate from captured originals')
        require(case['source']['sha256'] == source_sha
                and case['mac_manifest']['sha256'] == manifest_sha,
                name + ': shared Mac source or manifest identity changed')
        source = checked_file(root, case['source'])
        manifest = read_json(checked_file(root, case['mac_manifest']))
        require(manifest['reference'].get('capturedDocxSha256', source_sha) == source_sha,
                name + ': captured Mac input differs from shared rendered input')
        raw = source.with_name('pages.json.gz').read_bytes()
        require(hashlib.sha256(raw).hexdigest() == coordinates_sha
                == manifest['coordinatesSha256'], name + ': shared Mac coordinates changed')
        require(manifest['sourceSha256'] == source_sha
                and manifest['reference']['platform'] == 'mac'
                and manifest['reference']['application'] == 'Microsoft Word',
                name + ': wrong Mac reference')
        captured = json.loads(gzip.decompress(raw))
        expected = dict(page_count=len(captured), glyphs=[
            [i + 1, *g[:3]] for i, page in enumerate(captured) for g in page['body']])
        require(len(captured) == manifest['pages'] == pages
                and sum(not g[1].isspace() for g in expected['glyphs']) == glyphs,
                name + ': shared Mac glyph/page coverage changed')
        require(len(case['windows']) == 1
                and case['windows'][0]['font_source'] == 'original-word-pdf',
                name + ': missing original font-program provider')
        results.append((name, case, source, manifest, expected, glyphs))
    return results


def check_shared_originals(executable, output, root=ROOT):
    output.mkdir(parents=True, exist_ok=False)
    results = []
    for name, case, source, manifest, expected, glyphs in shared_originals(root):
        provider = case['windows'][0]
        archive, capture, document, _ = archive_reference(root, case, provider)
        target = output/name
        target.mkdir()
        font_pdf = target/'original-Windows-font-provider.pdf'
        with ZipFile(archive) as bundle:
            data = bundle.read(document['pdf'])
        require(hashlib.sha256(data).hexdigest() == document['pdf_sha256'],
                name + ': font provider PDF changed')
        font_pdf.write_bytes(data)
        fonts = target/'fonts'
        extract_fonts(font_pdf, fonts)
        actual_file = target/'native.json'
        painted = target/'native.pdf'
        arguments = [str(executable), str(source), str(actual_file), 'mac', str(painted),
                     '--fonts-directory', str(fonts)]
        if manifest.get('capturePaintedGlyphs') and manifest.get('clipPaintedGlyphs'):
            arguments.append('--painted-table-headers')
        subprocess.run(arguments, check=True)
        actual = read_json(actual_file)
        require_original_font_programs_registered(fonts, actual)
        require(assert_layout(expected, actual, case['source']['sha256'], name) == glyphs,
                name + ': shared Mac glyph coverage removed')
        for unclipped in (False, True):
            require(compare(expected, pdf_origins(painted, unclipped),
                            name + '/original-Mac-painted-PDF') == glyphs,
                    name + ': shared Mac PDF glyph coverage removed')
        results.append(dict(name=name, source_sha256=case['source']['sha256'],
                            Mac_reference=manifest['reference'],
                            Mac_coordinates_sha256=manifest['coordinatesSha256'],
                            font_program_provider_Windows_Word=capture['word'],
                            font_program_provider_archive=provider['archive'],
                            glyphs=glyphs, page_count=expected['page_count'],
                            native_table_glyph_projection=actual['table_glyph_projection']))
    require(len(results) == 89 and sum(r['glyphs'] for r in results) == 30096,
            'Shared Mac original coverage changed')
    print('89 shared Mac originals30096: layout and painted-PDF PASS', flush=True)
    return results


def require_mac_host(required):
    require(not required or sys.platform == 'darwin', 'Captured-original CI requires macOS')


def originals(root=ROOT):
    original = checked_file(root, dict(file=ORIGINAL, sha256=ORIGINAL_SHA256))
    control = checked_file(root, dict(file=CONTROL, sha256=CONTROL_SHA256))
    reference = checked_file(root, dict(file=REFERENCE, sha256=REFERENCE_SHA256))
    raw = read_json(reference)
    require(raw['docx_sha256'] == ORIGINAL_SHA256, 'Mac original capture input changed')
    require(raw['word_version'] == '16.114' and raw['word_build'] == '16.114.26101123',
            'Mac original Word version/build changed')
    require(raw['pdf_sha256'] == '079f2ff60c9a8731869fedd7c155b9c3e5870793d185aab7a3d0321761f5dd96',
            'Mac original PDF identity changed')
    require(original.read_bytes() != control.read_bytes(), 'Historical distinct inputs were collapsed')
    expected = dict(page_count=raw['page_count'], glyphs=[
        [page['page'], glyph['text'], glyph['x_pt'], row['baseline_pt']]
        for page in raw['pages'] for row in page['rows'] for glyph in row['characters']])
    require(expected['page_count'] == 1 and sum(not g[1].isspace() for g in expected['glyphs']) == 1070,
            'Mac original glyph/page coverage changed')
    return original, control, raw, expected


def assert_layout(expected, actual, source_sha256, label):
    require(actual['input_sha256'] == source_sha256, label + ': wrong rendered input')
    return compare(expected, actual, label + '/original-Mac-layout')


def check_native(executable, output, root=ROOT, *, require_macos=False):
    require_mac_host(require_macos)
    original, control, raw, expected = originals(root)
    index = read_json(root/'tests/word-oracle/windows/font-coordinates.json')
    provider, = [c for c in index['fixtures'] if c['name'] ==
                 'word-windows-floating-order-compat14-word16.0.19127.20936']
    require(provider['source']['sha256'] == CONTROL_SHA256
            and provider['word_product_version'] == '16.0.19127.20936'
            and provider['capture']['sha256'] == 'c4af57ec9ebdc3bc52b38791f01fc5a941f0177623b05aac670b3baebf51f8c4',
            'Original font-program provider changed')
    archive, capture, document, _ = archive_reference(
        root, {'source': provider['source']}, dict(provider, archive=provider['capture']))
    output.mkdir(parents=True, exist_ok=False)
    font_pdf = output/'original-Windows-font-provider.pdf'
    with ZipFile(archive) as bundle:
        data = bundle.read(document['pdf'])
    require(hashlib.sha256(data).hexdigest() == document['pdf_sha256'], 'Font provider PDF changed')
    font_pdf.write_bytes(data)
    fonts = output/'fonts'
    extract_fonts(font_pdf, fonts)
    results = []
    for tag, source, digest in [('captured-original', original, ORIGINAL_SHA256),
                                ('normalized-control', control, CONTROL_SHA256)]:
        actual_file = output/(tag+'.json')
        painted_pdf = actual_file.with_suffix('.pdf')
        subprocess.run([str(executable), str(source), str(actual_file), 'mac', str(painted_pdf),
                        '--fonts-directory', str(fonts)], check=True)
        actual = read_json(actual_file)
        require_original_font_programs_registered(fonts, actual)
        count = assert_layout(expected, actual, digest, tag)
        require(count == 1070, 'Mac original coordinate coverage removed')
        for unclipped in (False, True):
            require(compare(expected, pdf_origins(painted_pdf, unclipped),
                            tag+'/original-Mac-painted-PDF') == 1070, 'Mac PDF glyphs removed')
        results.append(dict(input=tag,source_sha256=digest,glyphs=count,page_count=actual['page_count']))
    a, b = [read_json(output/(tag+'.json')) for tag in ('captured-original','normalized-control')]
    require(a['glyphs'] == b['glyphs'] and a['page_count'] == b['page_count'],
            'Schema normalization changed Mac origins/pages')
    require((output/'captured-original.png').read_bytes() ==
            (output/'normalized-control.png').read_bytes(), 'Schema normalization changed Mac pixels')
    shared_results = check_shared_originals(executable, output/'shared-originals', root)
    result = dict(host=sys.platform,require_macos=require_macos,Mac_Word_version=raw['word_version'],
                  Mac_Word_build=raw['word_build'],Mac_capture_reference_sha256=REFERENCE_SHA256,
                  font_program_provider_Windows_Word=capture['word'],font_program_provider_archive=provider['capture'],
                  actual_native_executable_sha256=hashlib.sha256(executable.read_bytes()).hexdigest(),results=results,
                  shared_originals=shared_results,
                  scope='Actual distinct original and normalized Mac renders, each against1070 unchanged original Word glyphs plus real painted PDF on both clipping paths. Font programs from retained Windows Word PDF; original Mac PDF bytes not retained, original raw coordinates/digest preserved. No Word launch, fabricated expected positions, or snapshot update.')
    (output/'result.json').write_text(json.dumps(result,indent=2)+'\n',encoding='utf-8')
    print('Captured Mac original1070 and normalized control1070: layout/painted-PDF/PNG PASS',flush=True)
    return result


if __name__ == '__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-executable',type=Path,required=True)
    parser.add_argument('--output-directory',type=Path,required=True)
    parser.add_argument('--require-macos',action='store_true')
    args=parser.parse_args()
    check_native(args.native_executable.resolve(),args.output_directory.resolve(),require_macos=args.require_macos)
