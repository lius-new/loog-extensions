import base64
import gzip
import hashlib
import os
import pathlib
import tempfile
import time
import unittest
from agent import Agent
from search_agent import WorkerBundle

BUNDLE = pathlib.Path(os.environ.get('LOOG_SEARCH_BUNDLE', pathlib.Path(__file__).parent / 'search/target/bundle.gz'))


@unittest.skipUnless(BUNDLE.is_file(), 'build the bundled target scanner first')
class SearchTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix='loog-search-')
        self.agent = Agent()
        self.root = self.agent.mount(self.directory.name)['root']
        self.id = 0
        encoded = BUNDLE.read_bytes()
        binary = gzip.decompress(encoded)
        self.call('search_bundle_begin', size=len(binary), sha256=hashlib.sha256(binary).hexdigest())
        for offset in range(0, len(encoded), 128 * 1024):
            self.call('search_bundle_append', offset=offset, data=base64.b64encode(encoded[offset:offset + 128 * 1024]).decode())
        self.call('search_bundle_finish')

    def tearDown(self):
        children = [search.child for search in self.agent.searches.values()]
        self.agent.close()
        for child in children:
            child.wait(timeout=3)
        self.directory.cleanup()

    def call(self, method, **params):
        self.id += 1
        result = self.agent.handle({'id': self.id, 'method': method, 'params': {'root': self.root, **params}})
        self.assertNotIn('error', result, result)
        return result['result']

    def file(self, name, data):
        path = pathlib.Path(self.directory.name) / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data if isinstance(data, bytes) else data.encode())
        return str(path)

    def search(self, query='needle', buffers=(), **spec):
        options = {'spec': {'query': query, 'include': '', 'exclude': '', 'case_sensitive': True,
                    'whole_word': False, 'regex': False, 'include_hidden': False, 'include_ignored': False, **spec},
                   'encoding': {'auto_detect': True}, 'buffers': list(buffers)}
        handle = self.call('search', path=self.directory.name, options=options)['search']
        files, deadline = [], time.monotonic() + 10
        while True:
            page = self.call('search_next', search=handle, limit=32)
            files.extend(page['files'])
            if page['complete']:
                child = self.agent.searches[handle].child
                self.call('search_close', search=handle)
                child.wait(timeout=3)
                return files, page['progress']
            self.assertLess(time.monotonic(), deadline)
            time.sleep(.01)

    def test_target_names_encodings_ignore_rules_and_buffer_eligibility(self):
        self.file('.gitignore', 'ignored.txt\n*.skip\n!allowed.skip\n')
        self.file('ignored.txt', 'needle')
        self.file('allowed.skip', 'needle')
        self.file('blocked.skip', 'needle')
        self.file('.hidden.txt', 'needle')
        self.file('src/.ignore', 'private.txt\n')
        self.file('src/private.txt', 'needle')
        self.file('src/中文\\name.txt', '中文 needle\r\nnext')
        self.file('utf16.txt', 'needle 中文'.encode('utf-16'))
        self.file('gbk.txt', '这是一个用于验证编码识别的简体中文文件。needle 中文内容。'.encode('gbk'))
        self.file('binary', b'\x00\x01needle\x00')
        dirty = self.file('dirty.txt', b'\0do not decode disk')
        self.file('large.txt', b'x' * (4 * 1024 * 1024 + 1))
        os.symlink('/etc', os.path.join(self.directory.name, 'outside'))
        files, progress = self.search(buffers=[dirty])
        self.assertEqual({file['name'] for file in files}, {'allowed.skip', '中文\\name.txt', 'utf16.txt', 'gbk.txt', 'dirty.txt'})
        literal = next(file for file in files if file['name'] == '中文\\name.txt')
        self.assertEqual(literal['relative'], ['src', '中文\\name.txt'])
        self.assertEqual(literal['matches'][0]['range'], {'start': 7, 'end': 13})
        self.assertTrue(next(file for file in files if file['name'] == 'dirty.txt')['buffer'])
        self.assertEqual(progress['skipped_binary'], 1)
        self.assertEqual(progress['skipped_large'], 1)
        files, _ = self.search(include='src/*.txt', include_ignored=True)
        self.assertEqual({file['name'] for file in files}, {'private.txt', '中文\\name.txt'})

    def test_regex_zero_width_limits_cancellation_and_handle_scope(self):
        self.file('many.txt', 'needle\n' * 10010)
        files, progress = self.search()
        self.assertEqual(sum(len(file['matches']) for file in files), 10000)
        self.assertTrue(progress['limit_hit'])
        self.file('zero.txt', '中文\n')
        files, _ = self.search(query='^|$', regex=True, include='zero.txt')
        self.assertEqual(sum(len(file['matches']) for file in files), 3)
        options = {'spec': dict(query='needle', include='', exclude='', case_sensitive=True,
                    whole_word=False, regex=False, include_hidden=False, include_ignored=False),
                   'encoding': {}, 'buffers': []}
        handle = self.call('search', path=self.directory.name, options=options)['search']
        with tempfile.TemporaryDirectory(prefix='loog-search-other-') as other:
            other_root = self.agent.mount(other)['root']
            self.id += 1
            reply = self.agent.handle({'id': self.id, 'method': 'search_next', 'params': {'root': other_root, 'search': handle, 'limit': 1}})
            self.assertEqual(reply['error']['kind'], 'permission_denied')
        child = self.agent.searches[handle].child
        self.call('search_close', search=handle)
        child.wait(timeout=3)
        self.assertNotIn(handle, self.agent.searches)

    def test_integrity_failure_and_scope_escape_are_not_executed(self):
        encoded = BUNDLE.read_bytes()
        bundle = WorkerBundle(len(gzip.decompress(encoded)), '0' * 64)
        try:
            for offset in range(0, len(encoded), 128 * 1024):
                bundle.append(offset, base64.b64encode(encoded[offset:offset + 128 * 1024]).decode())
            with self.assertRaisesRegex(ValueError, 'integrity'):
                bundle.finish()
            self.assertFalse(bundle.ready)
        finally:
            bundle.close()
        self.id += 1
        reply = self.agent.handle({'id': self.id, 'method': 'search', 'params': {'root': self.root, 'path': '/etc', 'options': {}}})
        self.assertEqual(reply['error']['kind'], 'permission_denied')

    def test_cumulative_ignore_budget_is_visible(self):
        self.file('.gitignore', 'ignored\n' * 8193)
        self.file('visible.txt', 'needle')
        files, progress = self.search()
        self.assertEqual(files, [])
        self.assertTrue(progress['limit_hit'])
        self.assertIn('cumulative', progress['first_error'])

    def test_100_search_cancel_cycles_reap_children_and_descriptors(self):
        baseline = len(os.listdir('/proc/self/fd'))
        options = {'spec': dict(query='needle', include='', exclude='', case_sensitive=True,
                    whole_word=False, regex=False, include_hidden=False, include_ignored=False),
                   'encoding': {}, 'buffers': []}
        for _ in range(100):
            handle = self.call('search', path=self.directory.name, options=options)['search']
            child = self.agent.searches[handle].child
            self.call('search_close', search=handle)
            deadline = time.monotonic() + 3
            while self.agent.retired_searches:
                self.call('ping')
                self.assertLess(time.monotonic(), deadline)
                time.sleep(.001)
            # No test-side wait: the Agent itself must have reaped this child.
            with self.assertRaises(ChildProcessError):
                os.waitpid(child.pid, os.WNOHANG)
        self.assertEqual(len(os.listdir('/proc/self/fd')), baseline)
        self.assertFalse(self.agent.searches)


if __name__ == '__main__':
    unittest.main()
