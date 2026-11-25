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
    'probe-second-column-centered-table': (
        '5e40c4c74b7d6cb2a28ebd28113e20113904b390ab5d575f7c61bfbec92043c5',
        '3fb9a7536b937b02897fa33c9829bab77df54a6a9ba056386eb31e64c8beb1e0',
        '9642bbc326b0d42a7816231ba1d401025e2e750179a047a7ba7bbf7c45e7a723', 1, 127),
    'probe-table-flow-cell-spacing-start-column-1': (
        'f6ff8e146a871d65787334d77f33e3f333a4b4beec26f79eae250eca6121ae5d',
        'b750ebed9f32ac1f235787bf9c406b4ec105cfabb80ebf0e1372dc5559087584',
        'beecefa6f8c5f0ce0b51cb1eecc791b60ffe8f9b1371c471d5ddd5a6ba888b34', 2, 114),
    'probe-table-flow-continuous-start-column-1': (
        '1b4508e5c029d061df1e8c0085c94e280486caca837df868db72cacfb58a876b',
        'a9f529fde1fd546a596036bbec148e14c40aeefa3a6ac2b8e9faae32d7b865ea',
        'b4715943d684f5803c908d520ef935347439f9abc38912a37d47cc23ba957d5d', 2, 244),
    'probe-table-flow-continuous-start-column-2': (
        '5e7682b0608a62522999b7b9759406cec2cf4166a5bdbcfbb1cd5ca841ad39cd',
        'a80fde9a71b81ed2ed5dce0738ee51527c728ec7dfa6db7437a718c2f546810a',
        '0fdc2908e36963d4914b8eb958e95ab1cb96b342ead57c8a6cc8bb74e0e6ca80', 3, 237),
    'probe-width-equal-pct-2500-auto-0': (
        '36a591c6fbb97a37704a48a1bfff557e75ef02efa0c9e5af9bbdead16743f5e6',
        '7fefd2a3ede0faaf9cc7640289f51b83cf8bcf0dedafdb9de3fdb4627c9a57ec',
        '2976630f4e9f421d13c3053ec97c9e6952824efda04d5882aa897710334a788a', 2, 107),
    'probe-width-equal-pct-2500-dxa-2000': (
        'bd8fba61ca17839d2d6613925e3096e1102afea3427dd260cc1753936f57a86c',
        'e0623e87fc548f968a67880597e2f578fcf73f2d065a89bba328a223b29ff15d',
        '2976630f4e9f421d13c3053ec97c9e6952824efda04d5882aa897710334a788a', 2, 107),
    'probe-width-equal-pct-2500-pct-2500': (
        '15843562316b215a31394eca1e70165fe8fc76c4356bb687579278314e8cf6a3',
        '3730054a381f0d42849f950a24dbfc64cbfafd8f80a2b218e91bbd6c1fcc9211',
        '2976630f4e9f421d13c3053ec97c9e6952824efda04d5882aa897710334a788a', 2, 107),
    'probe-width-equal-pct-2500-pct-5000': (
        'ab820c0a8412845b005462a8402568430799428a2da7df90d08de4cddf496376',
        '5c4bf94485b745ce362ecbf216753c5d3dba3651dbce56d3c46a795c8bcfadae',
        '2976630f4e9f421d13c3053ec97c9e6952824efda04d5882aa897710334a788a', 2, 107),
    'probe-width-equal-pct-5000-pct-2500': (
        '481565abb6f1950da3904986542cb95fbedb564e43a0e6ff10b23f2e5b3ecf60',
        '5cdebae7e8ae60621a2e7b00e86d5d982a7cb6760e6c6fef477ef57c56015b97',
        '664310d26e900561a3b56991caeff51e2c67c5e8893351e6155ac6649e3dde30', 2, 107),
    'probe-table-flow-continuous-vertical-merge-start-column-1': (
        '4676dd4f7e155555b8688f5141677dde47b6d69470d5c1eb1cbc0a1904cca378',
        'af3b37320ae0fefa71427722c2025bbb585cdb8537b803b769694737d32a5310',
        'e3a278070c42dd740bc32145a411c9a359a1bf7cc5b79bc6f5c8daa4bb1d2b06', 2, 282),
    'probe-table-flow-continuous-vertical-merge-start-column-2': (
        '1597dfc6cb93d4fd97bf64899f76ac1b36e4fe97be29bad19cdac77457349d83',
        'c279d74ca975d268b33ede500441cee2d6088487aeb9e25d3b041720045beda1',
        '56c114badb0430a0d1f957212b5f210eb3f4eb4e449a41a247a2bacc2b73dee0', 3, 270),
    'probe-table-flow-split-rows-continuous-start-column-1': (
        'a47ad80e3904b49ed758c672faa0b7ccac3890db02d6288fd24fc3f32f8946e0',
        'f1a0ddbf3fb9fe4df688ab55dfa131005f412df8194e7577e16c491709dae744',
        '39e7a5e32de47fec2d11046dd3e816c08940be918785dba6b5323614a2caf321', 4, 752),
    'probe-table-flow-split-rows-start-column-1': (
        'a165ed52bb7cd7946c868e80c16938057f2328b838e1e07b3813b43adc54bac7',
        '9bef80a06b55791909f20d9dda11d78af410a6d107df9525c6e27cac2e4be8d5',
        '651dc339cb2f718b88bbf9c15930a6a9fd148f0a7e492f87fa2456b157e1ec5d', 4, 615),
    'probe-table-flow-split-rows-start-column-2': (
        '6f99c8f0650b376f4f02bb913e479d8f71a08502dbee4b9afcf7fd8a3ba8a32d',
        'ed5983aed16847fd7a5df6b11b484894a6be9c8632c1de5909a3ff8026077a54',
        '9cdc53f1e44c423f1541169836c0f3082c5fa87d65f3ab5db36e645c3f3c4476', 4, 615),
    'probe-table-flow-start-column-1': (
        'a2ea8cefe1f356dcb1a375e7f805fc942a926e7cd682ed6dce35d59adbb458c6',
        '14af706f357ec1aac10fc2e3eb707fbbfdeaba3b8bcf3a29638480a826d2ef43',
        'ee8c163838037d0981d6c642cf987bade768e3783595e3b82bec7bfb82ca1bfa', 2, 107),
    'probe-table-flow-start-column-2': (
        'e304ac753beb4f20232b3a57402a90648e8a94d11930f96ed876ec823fb0841a',
        '5d9f00cb863b475adebe81deb4eefebad19cab33a9fc195e0f433c9061bda51e',
        'ff8900d22e1276a51800dcf4d87b2649386a02362da7c7c9f9bac4e4ac8e62b0', 2, 107),
    'table-flow-continuous-start-column-1-mac': (
        '1b4508e5c029d061df1e8c0085c94e280486caca837df868db72cacfb58a876b',
        'a9f529fde1fd546a596036bbec148e14c40aeefa3a6ac2b8e9faae32d7b865ea',
        'b4715943d684f5803c908d520ef935347439f9abc38912a37d47cc23ba957d5d', 2, 244),
    'table-flow-continuous-vertical-merge-start-column-2-mac': (
        '1597dfc6cb93d4fd97bf64899f76ac1b36e4fe97be29bad19cdac77457349d83',
        '4c2a4ed0f961344f260bd9f219b1a8f84608635536baac8a8e6cea77fb71f327',
        '24800a0e47c5fb9465f98d4d58d7ef7f4ebb2ba6caefd7f03954465a8d129618', 3, 270),
    'table-flow-split-rows-start-column-1-mac': (
        'a165ed52bb7cd7946c868e80c16938057f2328b838e1e07b3813b43adc54bac7',
        '9bef80a06b55791909f20d9dda11d78af410a6d107df9525c6e27cac2e4be8d5',
        '651dc339cb2f718b88bbf9c15930a6a9fd148f0a7e492f87fa2456b157e1ec5d', 4, 615),
    'probe-unequal-variant-three-24-dxa': (
        '865961e8f2411afce1f4e844d095122bd819b0fcf68d6b32383c331927785038',
        '1dff6a00e5e1684c7f8c264143a8326f977ba3f29ea37635c70dcb6c73d9cdd8',
        '5aad7690d4cd2e80ad035e2a86be0e28af50bcf79a61a6c763ff2137bf8a1f88', 2, 233),
    'table-bidi-alignment-mac': (
        '8e0727a300cf0be7049465b33e3c4419d63034f4b104877922d33b4fde6cd6fd',
        'f6ad7501f2ea94d102334b7bcbf4672125929b7d256e24de1590aa469d17c387',
        'fe1f35d7e54e52ac4d5eddd19a68c550eac6ac858547743b90bee1af8e729ec8', 1, 561),
    'table-bidi-visual-mac': (
        '34b12018a851ce7ebc39bb7661297f6460fe54005e7950c59b6fd2a2e0318091',
        '4907f7d75d6fee91fd5d715dbad2fe49e484ea5edbe743046cd4dbf21683f46b',
        '532625fbbb052b33551b64af007b4d091b3a942354b0835c0d7ca08d9f08538e', 1, 492),
    'table-border-conflicts-mac': (
        '2130dd654c2d0138577e36d17c05f6c168be860f39735151e093f6e25248a9e8',
        '61c35516c4a8f33947bad4a7ecbe79dd9825ecfc233feb438f723be9a13c376f',
        '0af186d89d0d504f4116e318e9cec48f2b757d965971151821479cc272524b5c', 1, 980),
    'table-diagonal-borders-mac': (
        '50d7a59238ccd5afe3d19f1410e5293d81a307729abfed886e1a1970f4fe4c77',
        'cc7a2e11b2dc4110471e6ef34a5ca0d38f8974adeb833e6be68494e415424175',
        '086456524dbf85bae3021cbeaab5f579702365c4893dc5d6e3e4dc62ed5f60b9', 1, 309),
    'table-hidden-rows-mac': (
        '183ccd393fd54057d2472c3dc7c2e9d5280e98f3cd6846857913922c378919a1',
        '5b6f2be6a93ce17bf9b47ac32ef2e6b155f835866f49e63f02a71a9e61181f7a',
        '27f27ab96c8b8b51b2c92305387d95951d6eb94b1c784cb9631aebaace5ae829', 1, 407),
    'table-hide-mark-mac': (
        '9b026eb37bf4e52e845e7d2805e923985e5148de47fc2138dc418185780ad71e',
        'e72a3f538771164f49db23b5c3842ff65d836bde77947ff1f3db5cf65d5be5f1',
        '34ddee76b982376bfa60a90c41096b98c8eca47564cf0239bfa8296b3362079a', 1, 290),
    'table-repeating-header-mac': (
        'b0e1d64b757da296151e869eeaee17df29a67f4d0ce8e6955b5ce45c88f8840c',
        '28ad6548b9b20e689e51aa6e07c402d863c313f2261bb4f4c8f3ce66e8c5b276',
        '14837b89f9d390dd4bcbe526458aa2d15cc3b8cece564c00232e0102e49e28b8', 2, 1790),
    'probe-table-flow-unequal-continuous-right-three-columns-start-column-1': (
        'c23bf6d4bffb89bb1b286521d74fec4ce900578cd6734fe44436ce1f3631ef36',
        '37693c20d3dbaa02c8958ef2dfc09b09c71eb8c2bffe092ace67123508e26f60',
        'd6829390fba8f0be49892e98631e718b4bc6d29d20114e5bd020069f5f0ed5e2', 2, 267),
    'probe-table-flow-unequal-continuous-right-three-columns-start-column-2': (
        '4dcc7e73a0d104c505cdbefe24d319d655112d0ff583ddba507866987865a4b9',
        'e3e5b970b697c44f8df25355af0f38c38e9de28e488bb2374e0c41431d252c1d',
        '119c6dd6d1f045c01cc9c1acef4dd0856395bde882d65b34119a06caa5d22c22', 2, 264),
    'probe-table-flow-unequal-continuous-right-three-columns-start-column-3': (
        'ca37286c57328bf2e17ecebe2acd5d1d1058338061f361c2f2b68235a3b3633e',
        'a03077cf6340b58e1174cbc93ffa2c16af6de7e022c55e9a73e4e990ce81aba6',
        'f5cada04863298105964f513a423beae646222192ef10e62b2bb8ed31a30b20e', 2, 261),
    'probe-table-flow-unequal-right-start-column-1': (
        '341dd0851470c3463bcedac8f94a23872510a203db7c847a17421d38243133fc',
        '3f40b8f0d3bbf1b77c0d3bb4b11e4f77846b485eb1d99d33307486e106577ed9',
        'ad9e8b0a596c973b7e5810e896d17e804ec89c3df0355a1855674c60a6035231', 2, 215),
    'probe-table-flow-unequal-right-start-column-2': (
        '8204ce5ea70821dc387150c83a0364bf441fcf24e065f291497b79f15efe4650',
        '9d0a67e30db5ae6c72eac95d97c2567841386ed54fe4394f92cae1be7effab3d',
        '73cf312dff8e7e12c44335fe689a3011c935c905b551970af89e98e47a9c704e', 2, 215),
    'probe-unequal-left-narrow-first-1-9': (
        '9488256f0eeffa7c6006030943d3a6dd7b3581c299229c625d902ce74d387f9e',
        'da278969df545fc8aff89ab2dd33537460c0ad13865b8d2b30c6f7b68d21bd80',
        '631896ba65a6f349762c58e783fd796e3e664a699837187f8489cef7fd3d96bc', 1, 152),
    'probe-unequal-left-narrow-first-2-9': (
        'a91b6b8ebb5d8eb15a6f5755a7c5b6b9a278ba199add5bbf7c3e2216a694660b',
        '8eb32424ea11aab0a4304b3d41f59f53ac3566a3c4e7b3465232913f142bf98e',
        'f1102731e4f1721f93b2b7124c9c300afd2a52d2915a5d7a7a6836e6d56abcb1', 2, 152),
    'probe-unequal-left-start-column-1': (
        '4ba1d6d26cabfd9bc19c9e07aa56f7f7ddc47175fbf321da79282d1c394d8dd8',
        '598d143458fa71737f090fafaba6751ce167bac7531bf00f6a929197eea1c413',
        '7a90af92d0908462808d4c6fc58ffe5bde48ade467508409d9f0098b921a3837', 2, 215),
    'probe-unequal-left-wide-first-1-9': (
        'de03022e8907f323c73f8b384f3f133a2bf07ec49e2f0b73dcc11328e97e0fc1',
        'bced551c9a97c0105ebcaf5862057cd26bae60e0f5a34181a2090920ebe9339b',
        'd2bace23be4e7a64cb91678a50c637238c3bef8ce070f32ff5924db736ec8e77', 1, 152),
    'probe-unequal-left-wide-first-2-9': (
        'e4838211525e9ccf4a8bbbd97de8530c896de516e046dd12949f09f30bfe9f47',
        '5d74a2a9fd2d2925368346cd64b8e1c1dc372da5930cd33f0691c706c0c64402',
        '3228678eecb5bf0b9e2bcd404b6672664732b33242fa9659de9d143a6983b3a4', 2, 152),
    'probe-unequal-right-narrow-first-1-1': (
        '3e0a9cb676e8d3abefb41d490308e3d14ca99dd35a2b35adb55331748bef7867',
        '03d7a52c59b6c3452a7fa0b9a57975dec4beca0cd6e930415a882bf8c0580b28',
        '83d0eb3ea17ec143b2d200755e0e4882dba0ed7b439bc863ab348a4ae20941db', 1, 29),
    'probe-unequal-right-narrow-first-1-9': (
        '483c67194fc71ba25888b6b57fae60a1abce81580cb023257f12433ae07dd7e9',
        '718c568b4e88707959ab6136ea9e5702163cfd2c80917acc164946561af08708',
        '4f3dc889e4e9fe2a7c4b8d58e1a91715d58425ef40e64e5e6eed134fab5143b2', 1, 152),
    'probe-unequal-right-narrow-first-2-9': (
        '90085599cef3460d6f9608d0ba1d1fbd6c2e329084c1264a7ec941077747b21f',
        'c78862691dca594a43b39805c6ed187336af03dc4193feb9b58df3f9f1203c22',
        '4eec18cad6c9865acc4f4039b36bfd079f261fd2182387668fde5507282de142', 2, 152),
    'probe-unequal-right-three-10': (
        '72fd949f09a32668e2585718cfd119b11b0d77a3d5fca0d1ff411d0b45c63c81',
        '44311955753a1fad48b6e5fb70dde40e1ee2d4fa5ca77c705ab84e71f761ef16',
        'b452a001c2079547cf060918d7ee3fe8ae2d1b1d15a1ff33d63ebbc7a5536bc5', 1, 115),
    'probe-unequal-right-three-17': (
        '447cc7b5d861a8db4b667bf7fa707adee5ca13fedcbbfec26c360c2c858d3b2a',
        'c4846d64f2c3084c7a6b2ebe86b8c8f7e6fa3cc94bfec293ddcdb94bb6e5f7b0',
        'ba288ba5e6e7f29be88499d8529f587223a2be1b5c61e87778929b96d2df0d2a', 2, 174),
    'probe-unequal-right-three-24': (
        'b88365b763437910d6c7326246fbfd1cdbfe36f1f1ef4a28e967d66131733148',
        'a6ee737fa10740d3d8851a3949c16b16d6161e97d1c0e88f9c332c87f6be75fd',
        '88c6685aa8c4cd1db1421de9ebbd2e5a3b5add79f01ec7b8e5970c09cfadcc39', 2, 233),
    'probe-unequal-right-three-4': (
        '1e305f6c8ebe0d5427e9c7addc50dbb9650fd66d8fc048d29b90ac28e543eeb2',
        '733e046343230adfe60bca50282118536cfd04bba2aff45afa8e4fb51501e5b6',
        '24471d918eaf0fd72914dbd5e491bf3bff2ab2d6be88bbd0e4bcf521631a07e6', 1, 61),
    'probe-unequal-right-three-7': (
        '34ade372b13e94f2c17d2a472526d86e4d9c54b0927a2a7bde066bb546199cca',
        'cd4fac9d4501ee8720ebd4cfcc2b9606a72e33439535cd386ee2dcbb47b95434',
        'cfe60eb72b118ce0c82a73fb07cf689295e6b4a2c1945491ea5958aed27afba9', 1, 88),
    'probe-unequal-right-wide-first-1-1': (
        '2f21635c2210bee64dfc945b2a59620bb95c3a51c9fab75ed64e67f5a43a049b',
        'c18b96c324c7dc157d44c4cf646d5a1cab6971688bca019b96b4c58ae7e817c0',
        '8fcbe516fc916549b8a048c8b6b88b1556057087cb71f0b8ec50bca47a564903', 1, 29),
    'probe-unequal-right-wide-first-1-9': (
        'ada4f8928d001fbe3f2b677c6d947c494f8ba41c3bedffdbea5ab9e227cdc5b6',
        'bc2ac9bd48cfbf9fc605197db5bf3115a009e58cb0057249939a96dbb825041a',
        '24759fb1407acafbe28e1b8c24397e1bd74f155fef915b5b48d5f8e8e9ead05d', 1, 152),
    'probe-unequal-right-wide-first-2-9': (
        'e01afa56b4f11fcce52cdc4b39ed316cb16b5f41ed8d7f49e8c6296a1e1e565c',
        '7784972889928fcb7a2e7fece13ed6edd044a48cf2702f6de09ade758612ffb4',
        'def986867b2aec8255c4540f6427d7f0febce2fc7a0b4ad754a8c81cf82229e0', 2, 152),
    'probe-unequal-variant-narrow-first-2-9-left': (
        'c1035f8b07051813dfb80d5779264f184c6dc3a0e7ff331939e01cb6db25d3e0',
        '4fada5c58a7b0057c18d99260abef2d86f070770c7d92fee6b525c47f9e9bd9c',
        '902e668a48e17a9c0a3fb219fc17d43397e7bfeed0401b9857d08aae1550d0ab', 2, 152),
    'probe-unequal-variant-narrow-first-2-9-right': (
        '2a3280d08463acfe6a46436bb4ae3dcb2752ec5696fee8aa6feb804423ea9755',
        '3390231412c4574ba4ecb68c6f23bdcca3856dfc032f673beca9e1c3b4781e26',
        '4eec18cad6c9865acc4f4039b36bfd079f261fd2182387668fde5507282de142', 2, 152),
    'table-legacy-hmerge-mac': (
        'd7d32bbe5fc8baec326089fbff419b557732e80d86245794633a8a052feb7ffa',
        '7237b87fef1a0bcd81a0703552397cc5792b76d105b341bebac123341d2ec217',
        'ab7e294ff70cc631d9b9ffd59122921697c3df1708b573a90f04c9fd9ddd52a4', 1, 348),
    'table-cell-fit-text-mac': (
        '2f11b93894009a6aae32f06f831718f53924afe225860dd87d17acde14ec0ec1',
        '2904aa83d1ae4b57d28f8533f9979050efeb46d4516db9249cff0e4236bb0368',
        '65c50d531ecf005c2ee042086ad4378693d47149487e5cfcef66f5f45b31ac6d', 1, 351),

    'probe-width-single-pct-2500-pct-2500': ('f74cf7aeddf0c23de7f85904e62286aa9108cf7b114ba9265ba000b78640684c', '7cf5df14270473cf686059a4bd83b98dc0f711d4f556a139d9e8f1da362ae3f5', 'd0e012af76d5a4bd13392522517957f387c98fa8489b09bf0e4272597cee2bc3', 3, 107),
    'probe-width-unequal-dxa-2400-dxa-2000': ('a506ec0b61237aad385e39fc57e43d7d2c2fb17c700befeebe92805de4a4b98d', '0e6b492d96818ab8c8405d33d6cceda43ab5ce4282426775e4eedb34ac7efa97', 'bc48c94947aaebb6b1567bf627c90cb6f501edcc3390d6e49e4fc153773c0f8e', 2, 347),
    'probe-width-unequal-pct-2500-auto-0': ('598e3bf932f8ea79aeffe90748afbfba82eef1ddd627c838a25e5cbce2c264af', '99ddbb4ee7286d8b9f7abc41139358494a2a3ce28c782418e35885d1c2e362fb', 'aa0d326262cbc9dac0766328236e894ff08ed0ee1569efc884f2fe2abf7351e4', 2, 215),
    'probe-width-unequal-pct-2500-dxa-2000': ('8853a76fbdf8e3024c71d81830da307f5d73786edc271df564c295d3ef1381e7', '54e1c7a85faaebfb9fa55814801a3a81b8a3a387d5a7cb98195e66c3a44ccc3d', 'aa0d326262cbc9dac0766328236e894ff08ed0ee1569efc884f2fe2abf7351e4', 2, 215),

    'numbering-alignment-matrix-mac': ('73798fc698bd87d8b7dfaaa541ea7b85db3bb950e3e79ca2b75c936baab9fba8', '19773195d7b06ccec6a58f64e340b9f31f028f2de17e6103109ae75ce4eb75bb', '4583c70de14ef82cdfcea7dc67853ca95fc09ce50ada81a1db741900d379302f', 1, 729),
    'numbering-legal-format-matrix-mac': ('7219dd24ac737ad89b84a1758408a46a1a7c37f092722e08b8846ea411f02d92', 'd162bb2dbfc0949cd1920031e380f9c6dacd200dde8184926afdf69df7e2dfad', 'b889ce6249832986097dc7380436b686931a7a629c35a192667eae97547df2b3', 1, 414),
    'numbering-textbox-full-matrix-mac': ('3100cb3dd7a5a1901e4c2f9fc176b759b5b26e7c88db422a5c4a68cfc35a1b54', 'ff65d858eec98bf5bcdd3c93e616fab0850d0769ac44dc17491ed31562f157b7', '0e4e6e33388d2ea0b4ec31352fca541581a84e2eeba8f696d30fc53a4b47294c', 1, 729),

    'numbering-table-header-mac': ('9721cf7b8de1056124b3e851ea3173aacf92480ebbc364c7c320819c7044f43f', 'f44df7c386a1c8db1a52e1e1e8bfc99d4b3f5e4d30a43fef6fa955d2405d5c09', 'a2e007eadcfe5e6a49c8023fbbfe307ca7eb2019f9e056abd23cab3f70eb44b5', 2, 598),
    'numbering-table-nested-list-mac': ('a787347823b95e7a7d75fc6eb1fc998a3577ee9ddbcd7ee00fc9d99e6ff3d39a', 'a3ab1ce3674cd25ae4e58c77b94a449b8eee17ceab4d7ca3ad520a412b6dc626', 'b763cbae4f92d57bd8c26e2e5a8eff9f41ebb9e216146af01cfc8a02a625d6a7', 1, 450),
    'fixed-table-width-matrix-mac': ('5d53094641db91deb2ccee705807464bb889a8da0ae543ab96510d3941c8530d', 'dda853ca99f2fa98cfb937f4980a65fbacc67760ca0addfc431fff91e1eefdce', 'b6bf0a5bf399ec7c521ebbbef466221731164c07e14c7ca09e9bf214155943db', 8, 752),
    'numbering-table-matrix-mac': ('0a71ba6e7f2949f1e9cd3731523fcd5f6b1c678fe934abdb40dccfdcf22b00e8', '7e32ff9650d2dd66d6b0ca332465b52402df30f695c18e8bfeeb395bb59dabe5', '4583c70de14ef82cdfcea7dc67853ca95fc09ce50ada81a1db741900d379302f', 1, 729),
    'probe-table-flow-unequal-automatic-grid-start-column-1': ('912db14b4123e131b34cf6501b98c8d767adc5e51d5b0bd7d440d75a52d316f8', '11636e9c7f1573922b7495d922cea7a1b4cea0d25da24c114e61c95bb81bb8cc', 'f21f8e8374c8786be706b4bd4e30fa40d622db5436ed8d848ab7ea560e2e3b20', 2, 215),
    'probe-table-flow-unequal-automatic-grid-start-column-2': ('0628b15210382818608bccfb8f53ea556bd3133552eb1304be5a294437bd3336', 'ca2896eabad77ba81a95e745179a49778004b40d2066d4a3c0fdde4f108c8d80', '78a382c3f29283594a71304d78da086a1143db1351eb97339950714f33d3bcb7', 2, 215),
    'probe-unequal-variant-narrow-first-2-9-fixed': ('456c868001b488bc2cdf56ba21ebce276dca0515a4d0b41d1aa6524b4919b66a', '7ec873c5da032a2709dd8069e088cd823217c31b768a8a2e381866d36d15a9a8', 'd6bdef4bc1aaf164d25ad63d5eedd19a503fcb9e0c4bca70d180a59a311090f5', 2, 152),
    'probe-unequal-variant-three-24-fixed': ('a8ccfa52817c23e650cc95fc6a022b4dccd5c0224cf00f98205d2633c484f183', '172a2d5e41084ae19e99f31d5f67ce7bec48b8fadf2487e28f77aae173c0fab2', '58ab1e66028ee8872f7725764fb6a51b78afa80c1af08f1971ce435738793c7e', 2, 233),
    'probe-unequal-variant-narrow-first-2-9-no-header': ('6bd0c810d0f4a64c9a4c0c4121372286f4bf0ab1e5e7b55dd384e7519d2fb6fc', 'dccb126ed68df60b20645ff26f2917f0998a86f01d83e93fe05379a96338b3c3', '7fa22ae882e0927edbb7791cce4cbb61baecac77ef10091c0177e36513f37076', 2, 149),
    'probe-unequal-variant-three-24-no-header': ('b1d448be69c99cbb1992f3c1357474c23af72da7e46e8647235e17082a7c979e', '0749f489313f427ade6af5d0c01966b82867551f8a4e76756f6600747595dd19', '6ee18371a14c1c65d41a7d2cebbdb7053a27ed149debbef3fbdaf19e1a8a3721', 2, 221),
    'probe-unequal-continuation-border-0': ('62f30a28b1c7ca1a754be3d60667ec5b829fba75089dccd194b4ca121d87d359', '106b4536226f629781a3221a2d6815b23ffe826bed2ce7d9fe4580216730d274', 'feb6ae0980746822f5ffdf3586c35a23b62d7204c1306197fca8001fe59e2284', 2, 221),
    'probe-unequal-continuation-border-16': ('8f4d45418d483603d8b6dffd55905f750a576af30ad99d6398758d56721cea55', '34f60feb060a2a29e4b08f73e9fe41f4470ee4b441ca49a2f1d6175fc81e6125', 'fe0767b1cdd1095d3d6a3ed193a5381b74e93f71452625e102f6de1406e8f1f8', 2, 221),
    'probe-unequal-continuation-border-4': ('fce4a4535fb6e4b0148aef8fcc5e551ac12266291b2a592801f1e2de8de936e4', 'ca3755f44836b3e5160d41557513f24bc0cb1ae545755a2ef1c0f87595032241', '658402674bff275ced69ce4e8e7b406c7bf41e0f5732d699dd1af8abd47660be', 2, 221),
}


# These existing Mac math originals retain outline-only PDF text output.
# Assert their genuine UI origins independently; the 161 full PDF gates above
# remain mandatory. Windows fixes must preserve these Mac layout coordinates.
MATH_LAYOUT_ORIGINALS = {'probe-math-display-alignment-1': ('bc9962ebc9e6cf31110e0fcb818f3fe360459aa1c7f55c995f054cc23e697643',
                                    '5939a0071a6003f3a93605300809cb7a6966dcbd82efa0b7a461d4d44fc1f92d',
                                    '1b6331ba2466c1721cafb8ea5f386b9993de89bd4585d863ae3e10c4e1264a01',
                                    2,
                                    1046),
 'probe-math-display-rtl-wrap-1': ('aa22151e8ddcd0ff2c2c7e599b537621ec6633c5e2a30f9b3d5443edd635a82d',
                                   '6c92729ae5acccad1e7e0150a887e43c5f0e7f93bf72b4f7eaf1e62e86f060a2',
                                   'b8ae6ea8a54cedb6cd58dcb52fc77497c25ecc810f476566c9509d50ed4ebdc0',
                                   1,
                                   819),
 'probe-math-italic-advances-lowercase': ('b1dd36320fda67bbdc51f5e83ef5d2c2faa5b5d8857457c86f11a1e933a71f6a',
                                          '21806c73a57975e88f28376400a0a289b26656abfb1ffb99eb62f161ea62664f',
                                          'de4953ed02aa6cb04bcd29f9eadf220e65b05ea1efdf2a2ee8e2f419af7f9110',
                                          1,
                                          524),
 'probe-math-italic-advances-uppercase': ('a664dc4e8d5915c5d1425eb8961b0d8d01808c271d8225efa481585f4f60140a',
                                          '94fafe12787ce6e2e8f26ce9a5c2292d3df5b5b80c5614c10050a852e09687c3',
                                          'a4b863b81c8d0cbe677cc907167a22bb84ce29052fcde8e76ef1cf4742d6d461',
                                          1,
                                          524),
 'probe-math-paragraph-mark-size': ('f9b87f2e7e4254c026236f5ad5ecf7ad5e686c2015b0313b39c84c4eae91010c',
                                    '4975d6d08f5b2491460ace00696808891aa8b5290d2a0f834a166f9cec61b3d2',
                                    'b325916c0dedbc355c1f74ba2fb41ed5e40847173fdbfd0bb99a0a622bef87f4',
                                    1,
                                    184),
 'probe-math-relation-wrap-after': ('37193745f3098273fd613cb767222d6bc6ddf2342de37a8659dc76d1410fa928',
                                    'ad1acf1c68627d3652f204493b1676cf14d7cee1a575fd8b784eb89e742116e0',
                                    'e7910720e4713f540d215a7b0ab3de24ce01392ef7f98fa5a477f573ae1a1bcb',
                                    4,
                                    1009),
 'probe-math-relation-wrap-before': ('43c447432b5f6f038a987e958943f60e90625302e3ec76678badf53914489ba4',
                                     'be19c1726e05c151cff722b5a0ecd62a06c144cc0759512a0545e128d9402f9f',
                                     '7ba9c28cf16f23aa76b06cf932b0a1e3e3b146af73f5542b1495c40487a0ef60',
                                     4,
                                     1009),
 'probe-math-relation-wrap-repeat': ('c63d20b2e8db80c72f7bc2fcd1165978303ecf1e910fb57550e34fefc544a3c2',
                                     '3276597a0829566ee8b9d2bb8cc86a36fcf01dba2d58556044b92c47f5b8e1f4',
                                     'a658f59ca72a953db64ee0255b8b65050261d6f8d32574b1c737074172ddbd54',
                                     4,
                                     1028)}


MATH_LAYOUT_ORIGINALS.update({'probe-math-fraction-first-vs-prev': ('b4a18c1c87ecdb1b1177590d39ecb49452a6e312e7d82135af114992ac63de94',
                                       '3f27ce68ad5ee832ebc88ce98294b3d2716b3af0f61c9afeb108ca9aa856e5d8',
                                       'da570acc55b9bf4e7d22c1d2a78e156be9d73dbd05a5585745a939967d8183f0',
                                       1,
                                       84),
 'probe-math-fraction-larger-neighbors': ('12cc7422c9a73dfa28dae9ad91813616d0ea61f17f9f32e524c799d4cd16789f',
                                          'b971a9edf81b40d6bac858fe3ce14573cb835f5e25ce11f1d9b0b252b94b2d39',
                                          'b753151be0524e10a6eb59689f0b3a127095dd1066134877a34074e0aaee3bf0',
                                          1,
                                          76),
 'probe-math-fraction-leading': ('d2b2c2a764659bbb749d9d666fb1fa98ca385d36a936d47ad792ec138e671de2',
                                 '96fdedd6c6d4b1b422063054391a490a83b892502f8463179c80d2b30c2f0749',
                                 '3ddf1a4d7cdb367fdf92f87ccb312a3080f35d1d189819b56fefb4bda1f3e905',
                                 1,
                                 130),
 'probe-math-fraction-neighbors': ('fead76a10e66fe60ab830441098b072012072517a80e35c3ca9aba952d7802f9',
                                   'ca5088adecb14f7a3c5a548cfa6ed4803b44f2e0c58f8505d421b7e9dca38492',
                                   '855ee0333319356d0d11e1dc0714a7436a7e266fe9681e2f3632b5adb6739891',
                                   1,
                                   74),
 'probe-math-fraction-size-20': ('333a79b7219943ef896bdc299e60c2db52fd8d4be93414a07c60088f04105365',
                                 '9b193213dc8e133a9e5d7a8e9b935518586a3784a0a833f5a1767fd681136c2c',
                                 'da6227e9e332d8069ccf2a547fe50ad7a2da9ae0ef7278aa88615e084cfa2e07',
                                 1,
                                 54),
 'probe-math-fraction-size-24': ('6f799918bffe329264b7f3be4f5a2e31806b75ecd48e92407cf9f2d5a5de3520',
                                 '719c798968d39c44b7fc449e5ef3f4a50446aef6ed687fe37ea5acf7400607cd',
                                 '2d235aeb1f6f3608e349585406d723ffb1bebd1535072507731cc0b7d53f6a89',
                                 1,
                                 54),
 'probe-math-fraction-size-28': ('f4ac618c70f336280d5ef0ea0768164a5be308d8121c1afb58a12392562b5336',
                                 'b3d12effab67fef1de3c499b315d09ee3add57fe3c86fdf16a24a20d4ac67ef8',
                                 '2afcfee0003f76693e09e97a11d8046aa1dd1c8cbe412ec8625f691a3df71582',
                                 1,
                                 54),
 'probe-math-fraction-size-32': ('f0c4610086ac0a877fd9942d74a2b998b27a4acca011e43e1052889133347198',
                                 'a3ae7f5d9a180ccefeb063190656f6f566a8159722cee8127ae6edcd92bd67a8',
                                 'f296be5e4613a7ef28f38f1a2648a859538165d51cdae67ec23ae8515bd5972d',
                                 1,
                                 54),
 'probe-math-fraction-size-36': ('2c18765ea05dc2b312f21e4c1f6922a8c251a553c9f9f594b7809a2392463e08',
                                 'b48e16f4388198a2c88960b6fc72b639481b2846cb4a8aae5b788084f6c48b46',
                                 'e09939d120453fc63a9ba39b979b7ba5f0572af998ea62ab8120bef24bd8c067',
                                 1,
                                 54),
 'probe-math-fraction-size-40': ('f211e7cbce01868e6453bdaf02c7d197424f7e52aa508300e20956b6895dd165',
                                 '54d411422b897d181db02bffbab37277ba33ca82142235e24f9ba7b300ad5c1b',
                                 '218d89af5cb05e14f577a38d54b351e52f933fe25116fab9bbd7a6b59a633d3a',
                                 1,
                                 54),
 'probe-math-fraction-spacing-matrix': ('e50d530ac6d8c491ad7f90ffd771cb28ab4c6073282c9a136d98b85b9d4dfd0e',
                                        'be1191a1d0508ee356c115cda0b470903138d84f72e25859669064abb912fe7a',
                                        'a70e5d00e5d298f99cf9696ba14c9b4e4089c7612439e6e6d1b363b2c965db8d',
                                        1,
                                        162),
 'probe-math-repeat-bar-fraction-32': ('5900e52798d4dfd7ad83de4befca4f8386358af7a1812d53aec0ca69b9071f84',
                                       'f3093c4bb0b1b06fc58e415193a2824f0e33baf549618f51307c18e4b8364352',
                                       '3ddf1a4d7cdb367fdf92f87ccb312a3080f35d1d189819b56fefb4bda1f3e905',
                                       1,
                                       130),
 'probe-math-repeat-fraction-28': ('f5e0ff2577bfc793e7164db660095bdd9c97f36342b4f795fa72f0d8c2646e3a',
                                   '93262a820812d7510edde6a65468ccccb9c77f4012fc85973a3e51a6bb8ab0e6',
                                   '58a42402210d6449d96ae6b250b341aa5b5bf25df338027e27a2613fd00037a4',
                                   1,
                                   130),
 'probe-math-repeat-fraction-36': ('b94617104464b83c68682a39aec43e1a976c7eadfaa2951036421e6643ea8eaa',
                                   '2516b50308189a596b37eb5fe8f8e9709cdbaed0562ac327820018f2149a105a',
                                   '773982a7004d8e95db0fa3ce0a3466b8bb656084eed2ef300006e754363f6f91',
                                   1,
                                   130),
 'probe-math-repeat-fraction-40': ('7be3ba8a5c757b2f897b8c3e489bf744c55cbd668eff96492aba892526e7d83b',
                                   '890461f7e9b93faf22512e2686e14272aa59327f6a97f83c9354a8881b429f57',
                                   '4f24a9ab0b153b22eb60454f687d53b6b1b21d289e740e80c28370df6fba4c24',
                                   1,
                                   130),
 'probe-math-standalone-fractions-12': ('b8e369ffa2f7092558aaef422e9dffff6280079c0bd1482011982957145114b6',
                                        '19e9a602c00b463ee3072afb5d0bdcd9b0e6454ed0558ef6a9a0d45bc68fb198',
                                        '3bd8821c8d7e2568d560ef25c0e3c014f56189b3d30e4638e25f24f017ce3e36',
                                        1,
                                        142),
 'probe-math-standalone-fractions-explicit-sizes': ('b0054462f00eddb4741b3fb09200499f70c67760daaff0a63dd2adc0dfda7fcb',
                                                    '0d8bb74a92683a355ea14c41e195f6ef72b2a339d42ed4ee7e48eb5999467408',
                                                    'c1ea260a1256686eb7875430e8c486915c560e2bdfbb7d494205bcc2b5d7c25c',
                                                    1,
                                                    284)})

# Historical Mac traces include clipped glyph origins. Preserve that
# projection explicitly; these are additional logical gates, while all161
# existing shared originals retain their layout and painted-PDF assertions.
UNCLIPPED_TABLE_LAYOUT_ORIGINALS = {'probe-table-flow-unequal-continuous-vertical-merge-three-columns-rtl-section-start-column-1': ('8cebdc4679e907312a0afb64461d31fe0275cb8856dce54702107a2ab7cdb45b',
                                                                                                 '442bc9ee47b3191435d75314d0bb345c11f91ef626fe5944b6e68d30d53bf51c',
                                                                                                 '6dc851dd8995c8741927235b72338d52410b89822576258044be10ce6076bee3',
                                                                                                 2,
                                                                                                 526),
 'probe-table-flow-unequal-continuous-vertical-merge-three-columns-rtl-section-start-column-2': ('27199ed59be287089d55a4a3bc3709493a9689046914a8946b7421aa6bddca2c',
                                                                                                 '3f8081ccd60785ad2cf774c2a3de57ec569b60840331a2463e6e08a520824c20',
                                                                                                 'e138360f1423eb47d45d2f7b3c7691ed72ad0ea4b233a3d6d5bb9e23b93f288d',
                                                                                                 2,
                                                                                                 518),
 'probe-table-flow-unequal-continuous-vertical-merge-three-columns-rtl-section-start-column-3': ('eb733532cb1107ab112234789cd827f93a142a5254a23fd41a5af922fc76a445',
                                                                                                 '5dd8d2cc552073b0847fb352ddf158fea59aa780f03aefc73044852185318863',
                                                                                                 '6f93ac0eb146ab8a3d40b79749d1950f904a48fb607150f64a4fdc1e5e8265c9',
                                                                                                 2,
                                                                                                 510)}

THAI_LAYOUT_ORIGINALS = {'probe-complex-font-metrics-thonburi': ['a3624b218add7ffde685a1e84b699450711b3deb934e8d108a3540539ba90635',
                                         'ade3d196e9a967d74a593ca5b6e4880fb91b536d19163c732810c1ef88607f3c',
                                         'ad1889753120033bd90413673b1c844210d092b16285b8a379c47771b4f0e33f',
                                         3,
                                         75],
 'probe-complex-font-sweep-thonburi': ['a89fddbd946b44be2532215c542e3c47bcd5c17e77bca3be691f3fbab18b75ec',
                                       '6a9968c6b32939fdd2e7320d5120b8b29668966dcb598004a786b8ff51f82232',
                                       '734597cf9eef5572c13f4479d4e23a6357b4f49a1a311e9738eede67e3fe18fc',
                                       6,
                                       270]}

def shared_originals(root=ROOT, references=None):
    if references is None:
        references = SHARED_ORIGINALS
    registry = read_json(root/'tests/word-oracle/platform-coordinates.json')
    cases_by_name = {}
    for case in registry['fixtures']:
        cases_by_name.setdefault(case['mac_case'], []).append(case)
    # Reject missing/duplicate required cases before reading any original files.
    for name in references:
        require(len(cases_by_name.get(name, ())) == 1,
                name + ': missing or duplicate shared Mac original')
    results = []
    for name, (source_sha, manifest_sha, coordinates_sha, pages, glyphs) in references.items():
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
    require(len(results) == 161 and sum(r['glyphs'] for r in results) == 51810,
            'Shared Mac original coverage changed')
    print('161 shared Mac originals51810: layout and painted-PDF PASS', flush=True)
    return results


def check_math_layout_originals(executable, output, root=ROOT):
    output.mkdir(parents=True, exist_ok=False)
    results = []
    for name, case, source, manifest, expected, glyphs in shared_originals(root, MATH_LAYOUT_ORIGINALS):
        archive, capture, document, _ = archive_reference(root, case, case['windows'][0])
        target = output/name
        target.mkdir()
        with ZipFile(archive) as bundle:
            data = bundle.read(document['pdf'])
        require(hashlib.sha256(data).hexdigest() == document['pdf_sha256'],
                name + ': math font provider PDF changed')
        provider = target/'original-Windows-font-provider.pdf'
        provider.write_bytes(data)
        fonts = target/'fonts'
        extract_fonts(provider, fonts)
        actual_file = target/'native.json'
        subprocess.run([str(executable), str(source), str(actual_file), 'mac',
                        '--fonts-directory', str(fonts)], check=True)
        actual = read_json(actual_file)
        require_original_font_programs_registered(fonts, actual)
        require(assert_layout(expected, actual, case['source']['sha256'], name) == glyphs,
                name + ': Mac math original coverage changed')
        results.append(dict(name=name, source_sha256=case['source']['sha256'],
                            Mac_reference=manifest['reference'],
                            Mac_coordinates_sha256=manifest['coordinatesSha256'],
                            font_program_provider_Windows_Word=capture['word'],
                            font_program_provider_archive=case['windows'][0]['archive'],
                            glyphs=glyphs, page_count=expected['page_count']))
    require(len(results) == 25 and sum(r['glyphs'] for r in results) == 7939,
            'Mac math layout original coverage changed')
    print('25 genuine Mac math layout originals: PASS', flush=True)
    return results



def check_thai_layout_originals(executable, output, root=ROOT):
    output.mkdir(parents=True, exist_ok=False)
    results = []
    for name, case, source, manifest, expected, glyphs in shared_originals(root, THAI_LAYOUT_ORIGINALS):
        archive, capture, document, _ = archive_reference(root, case, case['windows'][0])
        target = output/name
        target.mkdir()
        with ZipFile(archive) as bundle:
            data = bundle.read(document['pdf'])
        require(hashlib.sha256(data).hexdigest() == document['pdf_sha256'],
                name + ': Thai font provider PDF changed')
        provider = target/'original-Windows-font-provider.pdf'
        provider.write_bytes(data)
        fonts = target/'fonts'
        extract_fonts(provider, fonts)
        actual_file = target/'native.json'
        subprocess.run([str(executable), str(source), str(actual_file), 'mac',
                        '--fonts-directory', str(fonts)], check=True)
        actual = read_json(actual_file)
        require_original_font_programs_registered(fonts, actual)
        require(assert_layout(expected, actual, case['source']['sha256'], name) == glyphs,
                name + ': Mac Thai original coverage changed')
        results.append(dict(name=name, source_sha256=case['source']['sha256'],
                            Mac_reference=manifest['reference'],
                            Mac_coordinates_sha256=manifest['coordinatesSha256'],
                            font_program_provider_Windows_Word=capture['word'],
                            font_program_provider_archive=case['windows'][0]['archive'],
                            glyphs=glyphs, page_count=expected['page_count']))
    require(len(results) == 2 and sum(r['glyphs'] for r in results) == 345,
            'Mac Thai layout original coverage changed')
    print('2 genuine Mac Thai layout originals: PASS', flush=True)
    return results


def check_unclipped_table_layout_originals(executable, output, root=ROOT):
    output.mkdir(parents=True, exist_ok=False)
    results = []
    for name, case, source, manifest, expected, glyphs in shared_originals(
            root, UNCLIPPED_TABLE_LAYOUT_ORIGINALS):
        require('includes clipped text origins' in manifest['reference'].get('extractor', '')
                and manifest.get('capturePaintedGlyphs')
                and not manifest.get('clipPaintedGlyphs'),
                name + ': original unclipped projection changed')
        archive, capture, document, _ = archive_reference(root, case, case['windows'][0])
        target = output/name
        target.mkdir()
        with ZipFile(archive) as bundle:
            data = bundle.read(document['pdf'])
        require(hashlib.sha256(data).hexdigest() == document['pdf_sha256'],
                name + ': original font provider PDF changed')
        provider = target/'original-Windows-font-provider.pdf'
        provider.write_bytes(data)
        fonts = target/'fonts'
        extract_fonts(provider, fonts)
        actual_file = target/'native.json'
        subprocess.run([str(executable), str(source), str(actual_file), 'mac',
                        str(actual_file.with_suffix('.pdf')), '--unclipped-tables',
                        '--fonts-directory', str(fonts)], check=True)
        actual = read_json(actual_file)
        require_original_font_programs_registered(fonts, actual)
        require(assert_layout(expected, actual, case['source']['sha256'], name) == glyphs,
                name + ': unclipped Mac original coverage changed')
        results.append(dict(name=name, source_sha256=case['source']['sha256'],
                            Mac_reference=manifest['reference'],
                            Mac_coordinates_sha256=manifest['coordinatesSha256'],
                            font_program_provider_Windows_Word=capture['word'],
                            font_program_provider_archive=case['windows'][0]['archive'],
                            glyphs=glyphs, page_count=expected['page_count'],
                            original_projection='unclipped-logical',
                            painted_PDF_reference_available=False))
    require(len(results) == 3 and sum(r['glyphs'] for r in results) == 1554,
            'Unclipped Mac table original coverage changed')
    print('3 genuine unclipped Mac table originals1554: logical layout PASS', flush=True)
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
    math_results = check_math_layout_originals(executable, output/'math-layout-originals', root)
    thai_results = check_thai_layout_originals(executable, output/'thai-layout-originals', root)
    unclipped_results = check_unclipped_table_layout_originals(
        executable, output/'unclipped-table-layout-originals', root)
    result = dict(host=sys.platform,require_macos=require_macos,Mac_Word_version=raw['word_version'],
                  Mac_Word_build=raw['word_build'],Mac_capture_reference_sha256=REFERENCE_SHA256,
                  font_program_provider_Windows_Word=capture['word'],font_program_provider_archive=provider['capture'],
                  actual_native_executable_sha256=hashlib.sha256(executable.read_bytes()).hexdigest(),results=results,
                  shared_originals=shared_results, math_layout_originals=math_results,
                  thai_layout_originals=thai_results, unclipped_table_layout_originals=unclipped_results,
                  scope='Actual distinct original and normalized Mac renders, each against1070 unchanged original Word glyphs plus real painted PDF on both clipping paths. The 161 shared originals retain all layout and painted-PDF gates. Two Thai originals additionally assert345 unchanged Mac glyph origins. Three further table originals assert1554 genuine unclipped logical origins without claiming a painted-PDF original. 25 math originals additionally assert7939 genuine Mac UI origins; their existing outline-only PDF text output is unchanged. Font programs from retained Windows Word PDF; original Mac PDF bytes not retained, original raw coordinates/digest preserved. No Word launch, fabricated expected positions, or snapshot update.')
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
