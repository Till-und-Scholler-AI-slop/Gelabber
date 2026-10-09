"""Checks of desktop/packaging/third-party-notices.py that need neither the
libwebrtc checkout nor a build: python3 -m unittest discover -s desktop/packaging/tests"""
import importlib.util
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest
from unittest import mock


SCRIPT = Path(__file__).parents[1] / 'third-party-notices.py'
SPEC = importlib.util.spec_from_file_location('third_party_notices', SCRIPT)
notices = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(notices)

APACHE = 'Apache License\n   Version 2.0, January 2004\n'
BSD = 'Redistribution and use in source and binary forms, with or without\nmodification\n'
MIT = (
    'Permission is hereby granted, free of charge, to any person obtaining a copy\n'
    'of this software and associated documentation files (the "Software"), to deal\n'
    'in the Software without restriction\n'
)


def tree(root, files):
    for name, text in files.items():
        path = Path(root) / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)
    return Path(root)


class KeptPartTests(unittest.TestCase):
    """Parts 2 and 3 are taken over from the existing file."""

    def file_with(self, body):
        rule = '#' * 78
        return f'title\n\n\n{rule}\n2. Native\n{rule}\n\n{body}\n\n\n{rule}\n3. Next\n{rule}\n\nlater\n'

    def made(self):
        return notices.native_part({}, 2, 'abc123', 'src', lambda _: 'first line\n\nISC\n', '--x', 'F')

    def kept(self, text):
        return notices.native_part(notices.kept_parts(text), 2, 'abc123', None, None, '--x', 'F')

    def test_an_untouched_part_is_kept_as_it_is(self):
        body = self.made()
        self.assertEqual(self.kept(self.file_with(body)), body)

    def test_a_changed_licence_identifier_is_refused(self):
        text = self.file_with(self.made()).replace('ISC', 'GPL-3.0')
        with self.assertRaisesRegex(SystemExit, 'part 2 of F is not the text the script wrote'):
            self.kept(text)

    def test_a_removed_entry_is_refused(self):
        text = self.file_with(self.made()).replace('first line\n\n', '')
        with self.assertRaisesRegex(SystemExit, 'not the text the script wrote'):
            self.kept(text)

    def test_other_inputs_ask_for_the_sources(self):
        parts = notices.kept_parts(self.file_with(self.made()))
        with self.assertRaisesRegex(SystemExit, 'inputs of part 2 changed'):
            notices.native_part(parts, 2, 'other', None, None, '--x', 'F')

    def test_a_marker_without_a_text_digest_is_not_taken_over(self):
        old = '[inputs abc123: the script keeps this part while they are unchanged]\n\nbody'
        self.assertEqual(notices.kept_parts(self.file_with(old)), {})


class NinjaRecordTests(unittest.TestCase):
    def deps_log(self, entries):
        """A .ninja_deps of version 4: {output: [inputs]}."""
        data, ids = b'# ninjadeps\n' + struct.pack('<i', 4), {}

        def path_id(path):
            nonlocal data
            if path not in ids:
                raw = path.encode() + b'\0' * (-len(path) % 4)
                data += struct.pack('<I', len(raw) + 4) + raw + struct.pack('<i', ~len(ids))
                ids[path] = len(ids)
            return ids[path]

        for output, inputs in entries.items():
            record = struct.pack(f'<{3 + len(inputs)}i', path_id(output), 0, 0, *map(path_id, inputs))
            data += struct.pack('<I', len(record) | 1 << 31) + record
        return data

    def test_sources_headers_and_assembler_includes_are_found(self):
        with tempfile.TemporaryDirectory() as tmp:
            src = tree(tmp, {
                'out/g/obj/lib/a.o': '', 'out/g/obj/lib/asm.o': '', 'out/g/obj/gen.o': '',
                'out/g/obj/lib/lib.ninja': (
                    'build obj/lib/a.o: cc ../../third_party/lib/a.c | obj/x.pcm\n'
                    'build obj/lib/never_built.o: cc ../../third_party/lib/b.c\n'
                    'build obj/gen.o: cxx gen/modules/portal/stubs.cc\n'
                ),
                'out/g/toolchain.ninja': 'build obj/lib/asm.o: __rule ../../third_party/lib/x86/sad.asm\n',
                'out/g/obj/lib/asm.o.d': (
                    'obj/lib/asm.o : \\\n  ../../third_party/lib/x86/sad.asm \\\n'
                    '  ../../third_party/lib//third_party/x86inc/x86inc.asm\n'
                ),
            })
            (src / 'out/g/.ninja_deps').write_bytes(self.deps_log({
                'obj/lib/a.o': [
                    '../../third_party/lib/a.c', '../../third_party/lib/third_party/vector/vector.h',
                    'obj/x.pcm', '/usr/include/stdio.h',
                ],
            }))
            objects = notices.ninja_build(src, 'out/g')
        self.assertEqual(objects, {
            'obj/lib/a.o': (
                'third_party/lib/a.c',
                {'third_party/lib/a.c', 'third_party/lib/third_party/vector/vector.h'},
            ),
            'obj/lib/asm.o': (
                'third_party/lib/x86/sad.asm',
                {'third_party/lib/x86/sad.asm', 'third_party/lib/third_party/x86inc/x86inc.asm'},
            ),
            'obj/gen.o': ('modules/portal/stubs.cc', {'modules/portal/stubs.cc'}),
        })

    def test_an_object_without_a_build_statement_stops_the_script(self):
        with tempfile.TemporaryDirectory() as tmp:
            src = tree(tmp, {'out/g/obj/a.o': ''})
            (src / 'out/g/.ninja_deps').write_bytes(self.deps_log({}))
            with self.assertRaisesRegex(SystemExit, 'no source in the checkout found for obj/a.o'):
                notices.ninja_build(src, 'out/g')


class LicenceFileTests(unittest.TestCase):
    """Which licence files a library entry prints."""

    FILES = {
        'LICENSE': 'webrtc',
        'api/a.cc': '',
        'third_party/lib/README.chromium': 'Name: lib\nLicense File: src/LICENSE, src/PATENTS\n',
        'third_party/lib/src/LICENSE': BSD,
        'third_party/lib/src/PATENTS': 'grant',
        'third_party/lib/src/a.c': '',
        'third_party/lib/src/third_party/vector/LICENSE': MIT,
        'third_party/lib/src/third_party/vector/vector.h': '',
        'third_party/lib/src/third_party/x86inc/LICENSE': 'ISC',
        'third_party/lib/src/third_party/x86inc/x86inc.asm': '',
        'third_party/lib/src/third_party/unused/LICENSE': 'not compiled',
        'third_party/lib/src/tools/notices.h': 'a header',
    }
    OBJECTS = {
        'obj/api/a.o': ('api/a.cc', {'api/a.cc', 'third_party/lib/src/third_party/vector/vector.h'}),
        'obj/lib/a.o': ('third_party/lib/src/a.c', {
            'third_party/lib/src/a.c', 'third_party/lib/src/third_party/vector/vector.h',
            'third_party/lib/src/tools/notices.h',
        }),
        'obj/lib/sad.o': ('third_party/lib/src/sad.asm', {
            'third_party/lib/src/sad.asm', 'third_party/lib/src/third_party/x86inc/x86inc.asm',
        }),
    }
    ROOTS = {'': 'webrtc', 'third_party/lib': 'lib'}

    def test_licence_files_of_compiled_directories_below_a_library_are_found(self):
        with tempfile.TemporaryDirectory() as tmp:
            src = tree(tmp, self.FILES)
            own, readers, users, found = notices.build_survey(src, self.OBJECTS, self.ROOTS)
            printed, left_out, below = notices.library_texts(
                src, 'lib', ['third_party/lib/src/LICENSE'],
                ['third_party/lib/src/LICENSE', 'third_party/lib/src/PATENTS'], found['lib'])
        self.assertEqual(own, {'webrtc': {'obj/api/a.o'}, 'lib': {'obj/lib/a.o', 'obj/lib/sad.o'}})
        self.assertEqual(readers, {'lib': {'obj/api/a.o'}})
        self.assertEqual(
            users['third_party/lib/src/third_party/vector'], {'obj/api/a.o', 'obj/lib/a.o'})
        self.assertEqual(found['webrtc'], ['LICENSE'])
        # README.chromium's second file is printed although the generator lacks it.
        self.assertEqual(printed, ['third_party/lib/src/LICENSE', 'third_party/lib/src/PATENTS'])
        self.assertEqual(left_out, {})
        self.assertEqual(below, [
            'third_party/lib/src/third_party/vector/LICENSE',
            'third_party/lib/src/third_party/x86inc/LICENSE',
        ])

    def test_a_file_of_an_unlisted_third_party_directory_stops_the_script(self):
        objects = {'obj/a.o': ('api/a.cc', {'api/a.cc', 'third_party/zlib/zlib.h'})}
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(SystemExit, 'third_party/zlib/zlib.h, which is in no library'):
                notices.build_survey(tree(tmp, self.FILES), objects, self.ROOTS)

    def test_files_next_to_the_licence_file_need_a_decision(self):
        files = {'third_party/lib/LICENSE': BSD + 'more', 'third_party/lib/COPYING.GPLv2': 'GPL',
                 'third_party/lib/LICENSE.md': 'more', 'third_party/lib/README.ijg': 'IJG'}
        with tempfile.TemporaryDirectory() as tmp:
            src = tree(tmp, files)
            with self.assertRaisesRegex(SystemExit, 'COPYING.GPLv2 stands next to the licence file'):
                notices.library_texts(src, 'lib', ['third_party/lib/LICENSE'], [], [])
            decided = {'third_party/lib/COPYING.GPLv2': 'Not this build.', 'third_party/lib/README.ijg': True}
            with mock.patch.object(notices, 'BESIDE_THE_LICENCE', decided):
                printed, left_out, below = notices.library_texts(
                    src, 'lib', ['third_party/lib/LICENSE'], [], ['third_party/lib/LICENSE.md'])
        self.assertEqual(printed, ['third_party/lib/LICENSE', 'third_party/lib/README.ijg'])
        self.assertEqual(left_out, {
            'Not this build.': ['COPYING.GPLv2'],
            'Its text is part of third_party/lib/LICENSE, printed here.': ['LICENSE.md'],
        })
        self.assertEqual(below, [])

    def test_source_files_are_not_licence_files(self):
        for name in ('LICENSE', 'LICENSE.md', 'PATENTS', 'COPYING.LGPLv2.1', 'license_template.txt'):
            self.assertTrue(notices.licence_named(name), name)
        for name in ('notices.h', 'license.py', 'README.ijg', 'AUTHORS'):
            self.assertFalse(notices.licence_named(name), name)


class ComparisonTests(unittest.TestCase):
    """README.chromium's licence names against the wording of the printed files."""

    def compared(self, declared, *texts):
        return notices.compared('lib', declared, [notices.plain(text) for text in texts])

    def test_names_without_their_wording_and_wording_without_a_name_are_reported(self):
        said = self.compared('MIT, BSD-3-Clause, OpenSSL, ISC, SSLeay', APACHE + BSD)
        self.assertIn('names MIT, OpenSSL, ISC and SSLeay; the wording of these was not found', said)
        self.assertIn('holds the wording of Apache-2.0, which README.chromium does not name', said)

    def test_matching_names_and_texts_are_not_remarked_on(self):
        self.assertEqual(self.compared('BSD-2-Clause, Patent', BSD, 'Additional IP Rights Grant (Patents)'), '')
        self.assertEqual(self.compared('Apache-2.0', ' * Apache License,\n * Version 2.0\n'), '')
        self.assertEqual(self.compared('Custom license', 'do what you like'), '')

    def test_mit_and_ncsa_are_told_apart(self):
        ncsa = MIT.replace('in the Software', 'with the Software')
        self.assertIn('names MIT;', self.compared('MIT, NCSA', ncsa))
        self.assertEqual(self.compared('MIT, NCSA', ncsa, MIT), '')

    def test_a_permission_notice_without_fee_clause_is_not_isc(self):
        text = 'Permission to use, copy, modify, and distribute this software for\nany purpose without fee'
        self.assertEqual(self.compared('Custom license', text), '')

    def test_an_unknown_licence_name_stops_the_script(self):
        with self.assertRaisesRegex(SystemExit, 'NAMED_LICENCES'):
            self.compared('WTFPL', BSD)


class SystemHeaderTests(unittest.TestCase):
    def test_inline_functions_of_sysroot_headers_are_found_by_symbol(self):
        header = (
            '/* Simple Plugin API\n *\n * Copyright (c) 2018 Somebody\n */\n'
            'static inline int spa_pod_builder_add(struct b *b) { return 0; }\n'
            'static inline int spa_never_used(void) { return 0; }\n'
        )
        library = {'_ZL19spa_pod_builder_addP1b': 't', 'spa_never_used': 'U', 'cxx_only': 't'}
        with tempfile.TemporaryDirectory() as tmp:
            src = tree(tmp, {
                'build/linux/debian_x_amd64-sysroot/usr/include/spa-0.2/spa/pod/builder.h': header,
                'build/linux/debian_x_amd64-sysroot/usr/include/c++/10/x.h':
                    'static inline int cxx_only(void) { return 0; }\n',
            })
            block = notices.system_headers(src, library)
        self.assertIn('[usr/include/spa-0.2/spa/pod/builder.h]\nFunctions: spa_pod_builder_add.\n', block)
        self.assertIn('Simple Plugin API\n\nCopyright (c) 2018 Somebody\n', block)
        self.assertNotIn('spa_never_used', block)
        self.assertNotIn('cxx_only', block)


class CheckTests(unittest.TestCase):
    """--check on the committed file, where the script's own needs are met
    (Linux x86-64, the pinned toolchain, the crates of the lock file)."""

    def check(self, path):
        return subprocess.run(
            [sys.executable, str(SCRIPT), '--check', '--offline', '--output', str(path)],
            capture_output=True, text=True)

    def test_a_native_part_changed_by_hand_fails_the_check(self):
        committed = notices.OUTPUT.read_text(encoding='utf-8')
        with tempfile.TemporaryDirectory() as tmp:
            copy = Path(tmp) / notices.OUTPUT.name
            copy.write_text(committed, encoding='utf-8')
            result = self.check(copy)
            stale = ('is out of date', 'is not the text the script wrote', 'the inputs of part')
            if result.returncode and not any(reason in result.stderr for reason in stale):
                self.skipTest(result.stderr.strip())
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn('  Licence   ISC\n', committed)
            copy.write_text(committed.replace('  Licence   ISC\n', '  Licence   GPL-3.0\n'), encoding='utf-8')
            result = self.check(copy)
            self.assertEqual(result.returncode, 1)
            self.assertIn('part 3 of THIRD-PARTY-NOTICES.txt is not the text the script wrote', result.stderr)


if __name__ == '__main__':
    unittest.main()
