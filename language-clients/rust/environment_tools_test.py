"""Isolated installer tests; all downloads are replaced by in-memory fixtures."""
import concurrent.futures
import gzip
import hashlib
import io
import os
import stat
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import environment_tools as tools


def tar(entries):
    output = io.BytesIO()
    with tarfile.open(fileobj=output, mode='w:gz') as archive:
        for name, data in entries:
            item = tarfile.TarInfo(name)
            if isinstance(data, bytes):
                item.size = len(data)
                archive.addfile(item, io.BytesIO(data))
            else:
                item.type, item.linkname = tarfile.SYMTYPE, data
                archive.addfile(item)
    return output.getvalue()


class Downloads:
    def __init__(self, files):
        self.files, self.calls = files, []

    def open(self, url, timeout):
        self.calls.append(url)
        return io.BytesIO(self.files[url])


class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='loog-tools-test-')
        self.addCleanup(self.temporary.cleanup)
        self.cache = self.temporary.name
        self.payload = gzip.compress(b'#!/bin/sh\nexit 0\n')
        self.url = 'https://example.invalid/server.gz'
        self.spec = {'id': 'example.language', 'kind': 'native', 'platforms': {'linux-x86_64': {
            'url': self.url, 'format': 'gz', 'sha256': hashlib.sha256(self.payload).hexdigest()}}}
        self.downloads = Downloads({self.url: self.payload})
        context = patch.object(tools.urllib.request, 'build_opener', return_value=self.downloads)
        context.start()
        self.addCleanup(context.stop)

    def prepare(self):
        return tools.prepare(self.spec, self.cache, 'Linux', 'x86_64')

    def test_native_install_is_pinned_reusable_and_does_not_overwrite_damaged_cache(self):
        directory = self.prepare()
        program = os.path.join(directory, 'server')
        with open(program, 'rb') as source:
            self.assertEqual(source.read(), gzip.decompress(self.payload))
        self.assertTrue(os.stat(program).st_mode & stat.S_IXUSR)
        self.assertEqual(self.prepare(), directory)
        self.assertEqual(len(self.downloads.calls), 1)
        with open(program, 'wb') as output:
            output.write(b'damaged')
        with self.assertRaisesRegex(ValueError, 'integrity'):
            self.prepare()
        with open(program, 'rb') as source:
            self.assertEqual(source.read(), b'damaged')
        self.assertEqual(len(self.downloads.calls), 1)

    def test_concurrent_installations_share_one_completed_cache(self):
        with concurrent.futures.ThreadPoolExecutor(2) as pool:
            values = list(pool.map(lambda _: self.prepare(), range(2)))
        self.assertEqual(values[0], values[1])
        self.assertEqual(len(self.downloads.calls), 1)

    def test_failed_hash_cleans_only_its_new_staging_directory(self):
        self.spec['platforms']['linux-x86_64']['sha256'] = '00' * 32
        with self.assertRaisesRegex(ValueError, 'integrity'):
            self.prepare()
        parent = os.path.join(self.cache, 'loog-language-tools', self.spec['id'])
        self.assertEqual(os.listdir(parent), ['.install.lock'])

    def test_node_and_locked_server_packages_preserve_their_own_paths(self):
        runtime = tar([('node/bin/node', b'node-program'), ('node/LICENSE', b'node-license')])
        package = tar([('package/bin/server.js', b'process.stdin.resume()'), ('package/LICENSE', b'license')])
        node_url, package_url = 'https://example.invalid/node.tgz', 'https://example.invalid/server.tgz'
        self.downloads.files.update({node_url: runtime, package_url: package})
        self.spec.update({'kind': 'node', 'entry': 'node_modules/server/bin/server.js',
            'platforms': {'linux-x86_64': {'url': node_url, 'sha256': hashlib.sha256(runtime).hexdigest(),
                'member': 'node/bin/node', 'license_member': 'node/LICENSE'}},
            'packages': [{'path': 'node_modules/server', 'url': package_url,
                'integrity': 'sha512-' + tools.base64.b64encode(hashlib.sha512(package).digest()).decode()}]})
        directory = self.prepare()
        self.assertTrue(os.path.isfile(os.path.join(directory, self.spec['entry'])))
        self.assertTrue(os.path.isfile(os.path.join(directory, 'NODE-LICENSE')))
        self.assertEqual(self.prepare(), directory)
        self.assertEqual(len(self.downloads.calls), 2)

    def test_archives_reject_traversal_links_duplicates_and_excessive_expansion(self):
        for entries in [
            [('package/../../escape', b'x')], [('/absolute', b'x')],
            [('package/link', '/outside')], [('package/x', b'x'), ('package/x', b'y')],
        ]:
            with tempfile.TemporaryDirectory(dir=self.cache) as root:
                archive = os.path.join(root, 'archive')
                with open(archive, 'wb') as output:
                    output.write(tar(entries))
                with self.assertRaises(ValueError):
                    tools.unpack(archive, tools.Budget(root), 'node_modules/example')
        with patch.object(tools, 'MAX_EXPANDED', 4):
            with self.assertRaisesRegex(ValueError, 'budget'):
                self.prepare()

    def test_unsupported_targets_and_non_https_sources_fail_before_installing(self):
        for system, machine in [('Windows', 'x86_64'), ('Linux', 'unknown')]:
            with self.assertRaisesRegex(ValueError, 'Linux'):
                tools.prepare(self.spec, self.cache, system, machine)
        self.spec['platforms']['linux-x86_64']['url'] = 'http://example.invalid/server.gz'
        with self.assertRaisesRegex(ValueError, 'HTTPS'):
            self.prepare()
        self.assertEqual(self.downloads.calls, [])

    def test_cache_budget_counts_interrupted_installs_without_removing_them(self):
        parent = os.path.join(self.cache, 'loog-language-tools', self.spec['id'])
        os.makedirs(parent, mode=0o700)
        for index in range(tools.MAX_CACHE_ENTRIES):
            os.mkdir(os.path.join(parent, 'old.partial-' + str(index)))
        with self.assertRaisesRegex(ValueError, 'interrupted'):
            self.prepare()
        self.assertEqual(len(os.listdir(parent)), tools.MAX_CACHE_ENTRIES + 1)
        self.assertEqual(self.downloads.calls, [])

    def test_disk_budget_and_cache_links_fail_before_downloading(self):
        parent = os.path.join(self.cache, 'loog-language-tools', self.spec['id'])
        os.makedirs(parent, mode=0o700)
        old = os.path.join(parent, 'old')
        with open(old, 'wb') as output:
            output.write(b'unchanged')
        with patch.object(tools, 'MAX_CACHE_BYTES', tools.MAX_EXPANDED + tools.MAX_DOWNLOAD):
            with self.assertRaisesRegex(ValueError, 'size budget'):
                self.prepare()
        os.symlink(old, os.path.join(parent, 'link'))
        with self.assertRaisesRegex(ValueError, 'unexpected file type'):
            self.prepare()
        with open(old, 'rb') as source:
            self.assertEqual(source.read(), b'unchanged')
        self.assertEqual(self.downloads.calls, [])


if __name__ == '__main__':
    unittest.main()
