"""Offline Linux integration: fake CLI, HTTP boundaries, real installer with fake OS tools."""
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import unittest
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[1]


def script(path, text):
    path.write_text(text, encoding="utf-8")
    path.chmod(0o755)


@unittest.skipUnless(os.name == "posix", "Linux integration")
class ZeroTierHTTP(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory()
        cls.work = Path(cls.temp.name)
        cls.calls = cls.work / "calls"
        cli = cls.work / "cli"
        script(cli, f'''#!/usr/bin/python3
import json,sys
with open({str(cls.calls)!r},'a') as f: f.write(' '.join(sys.argv[1:])+'\\n')
assert sys.argv[1]=='-j'
data={{'info':{{'address':'abcdef0123','online':True,'authtoken':'SECRET'}},
'listnetworks':[{{'id':'1234567890abcdef','name':'<img src=x onerror=alert(1)>','status':'OK','assignedAddresses':['10.0.0.1/24'],'secret':'SECRET'}}],
'listpeers':[{{'address':'0123456789','role':'PLANET','latency':-1}}]}}
print(json.dumps(data[sys.argv[2]]))
''')
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        cls.base = f"http://127.0.0.1:{port}"
        env = dict(os.environ, VPN_UI_ZEROTIER_COMMAND=str(cli),
                   VPN_UI_LISTEN=f"127.0.0.1:{port}", VPN_UI_GEOIP_CACHE=str(cls.work / "geo.json"))
        cls.log = (cls.work / "server.log").open("w")
        cls.proc = subprocess.Popen([str(ROOT / "target/release/vpn-ui")], env=env, stdout=cls.log, stderr=cls.log)
        for _ in range(100):
            try:
                urllib.request.urlopen(cls.base + "/healthz", timeout=1).close()
                return
            except OSError:
                time.sleep(.1)
        raise RuntimeError("server did not start")

    @classmethod
    def tearDownClass(cls):
        cls.proc.terminate()
        cls.proc.wait(timeout=10)
        cls.log.close()
        cls.temp.cleanup()

    def get(self, path):
        return urllib.request.urlopen(self.base + path, timeout=15)

    def test_monitoring_cache_and_escape(self):
        with self.get('/zerotier') as response:
            html = response.read().decode()
            self.assertEqual(response.headers['Cache-Control'], 'no-store')
        self.assertIn('&lt;img', html)
        self.assertNotIn('<img', html)
        self.assertNotIn('SECRET', html)
        self.assertNotIn('href="/amneziawg"', html)
        with self.get('/api/zerotier/status') as response:
            body = response.read().decode()
            data = json.loads(body)
            self.assertEqual(response.headers['Cache-Control'], 'no-store')
        self.assertTrue(data['node']['online'])
        self.assertNotIn('SECRET', body)
        self.assertEqual(self.calls.read_text().splitlines(), ['-j info', '-j listnetworks', '-j listpeers'])

    def test_no_mutation_or_awg2_routes(self):
        for route, method, code in [('/api/zerotier/status', 'POST', 405), ('/zerotier', 'POST', 405),
                                    ('/amneziawg', 'GET', 404), ('/api/amneziawg/status', 'GET', 404),
                                    ('/api/admin/amneziawg/manage', 'POST', 404)]:
            with self.assertRaises(urllib.error.HTTPError) as caught:
                urllib.request.urlopen(urllib.request.Request(self.base + route, method=method), timeout=5)
            self.assertEqual(caught.exception.code, code)


@unittest.skipUnless(os.name == "posix", "Linux shell integration")
class Installer(unittest.TestCase):
    def run_install(self, fail=False):
        with tempfile.TemporaryDirectory() as tmp:
            work = Path(tmp)
            commands = work / 'commands'
            commands.mkdir()
            (work / 'target/release').mkdir(parents=True)
            (work / 'target/release/vpn-ui').write_text('NEW')
            binary = work / 'vpn-ui'
            script(binary, 'OLD')
            wrapper = work / 'wrapper'
            sudoers = work / 'sudoers'
            log = work / 'calls'
            script(commands / 'id', '#!/bin/sh\n[ "$1" = -u ] && echo 0\nexit 0\n')
            script(commands / 'visudo', '#!/bin/sh\nexit 0\n')
            script(commands / 'sleep', '#!/bin/sh\nexit 0\n')
            script(commands / 'install', '#!/usr/bin/python3\nimport os,sys\na=sys.argv[1:]\nwhile \"-o\" in a:\n i=a.index(\"-o\"); del a[i:i+2]\nwhile \"-g\" in a:\n i=a.index(\"-g\"); del a[i:i+2]\nos.execv(\"/usr/bin/install\", [\"install\"]+a)\n')
            script(commands / 'systemctl', f'''#!/bin/sh
echo "$*" >> {log}
if [ "$1" = restart ] && [ "$(cat {binary})" = NEW ] && [ {int(fail)} = 1 ]; then exit 1; fi
exit 0
''')
            script(commands / 'curl', f'''#!/usr/bin/python3
import sys,json
from pathlib import Path
args=sys.argv[1:]
url=next(x for x in args if x.startswith('http'))
if url.endswith('healthz'):
 print(json.dumps({{'version':'0.8.8' if Path({str(binary)!r}).read_text()=='NEW' else '0.8.7'}}))
elif url.endswith('/api/zerotier/status'): print(json.dumps({{'node':None,'networks':None,'peers':None,'errors':['missing']}}))
elif url.endswith('/zerotier'): print('zt-grid')
''')
            source = (ROOT / 'deploy/install-zerotier-v088.sh').read_text()
            source = source.replace('/usr/local/bin/vpn-ui-zerotier-ro', str(wrapper))
            source = source.replace('/usr/local/bin/vpn-ui', str(binary))
            source = source.replace('/etc/sudoers.d/vpn-ui-zerotier', str(sudoers))
            source = source.replace('/root/vpn-ui-backups', str(work / 'backups'))
            source = source.replace('/run/lock/vpn-ui-v088.lock', str(work / 'lock'))
            source = source.replace('> "$BACKUP/zerotier.html"', '> "$BACKUP/zerotier.html"')
            # Fixture keeps the curl output redirections and runs the actual rollback code.
            candidate = work / 'installer'
            candidate.write_text(source)
            (work / 'deploy/bin').mkdir(parents=True)
            (work / 'deploy/sudoers').mkdir()
            for rel in ['bin/vpn-ui-zerotier-ro', 'sudoers/vpn-ui-zerotier']:
                (work / 'deploy' / rel).write_bytes((ROOT / 'deploy' / rel).read_bytes())
            result = subprocess.run(['bash', str(candidate), hashlib.sha256(b'NEW').hexdigest()], cwd=work,
                                    env=dict(os.environ, PATH=str(commands) + ':' + os.environ['PATH']),
                                    capture_output=True, text=True, timeout=20)
            self.assertEqual(result.returncode, 1 if fail else 0, result.stdout + result.stderr)
            self.assertEqual(binary.read_text(), 'OLD' if fail else 'NEW')
            self.assertEqual(wrapper.exists(), not fail)
            self.assertEqual(sudoers.exists(), not fail)
            calls = log.read_text().splitlines()
            self.assertTrue(all(line == 'restart vpn-ui.service' for line in calls if line.startswith('restart')))

    def test_success(self):
        self.run_install()

    def test_restart_failure_rolls_back_binary_and_permissions(self):
        self.run_install(fail=True)

    def test_wrapper_rejects_mutation(self):
        for args in [[], ['-j', 'join'], ['-j', 'info', 'extra'], ['-Tsecret', 'info']]:
            result = subprocess.run(['sh', str(ROOT / 'deploy/bin/vpn-ui-zerotier-ro'), *args])
            self.assertEqual(result.returncode, 64)


if __name__ == '__main__':
    unittest.main()
