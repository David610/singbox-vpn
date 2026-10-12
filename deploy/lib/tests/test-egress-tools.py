#!/usr/bin/env python3
import importlib.util
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import tempfile
import time
sys.dont_write_bytecode = True
import struct
import unittest
from unittest.mock import patch


def load(name):
    path = Path(__file__).resolve().parents[1] / (name + ".py")
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


families = load("egress_families")
front = load("literal_ip_socks_front")
lab = load("egress_family_lab")


class Tools(unittest.TestCase):
    def test_direction_process_visibility_and_mapped_peers(self):
        rows = '\n'.join([
            'tcp ESTAB 0 0 [::]:443 [::ffff:8.8.8.8]:51500 users:(("sing-box",pid=10,fd=1))',
            'tcp ESTAB 0 0 [::ffff:127.0.0.1]:51000 [::ffff:8.8.8.8]:443 users:(("sing-box",pid=10,fd=2))',
            'udp ESTAB 0 0 [::1]:51001 [2606:4700:4700::1111]:443 users:(("sing-box",pid=10,fd=3))',
            'tcp ESTAB 0 0 127.0.0.1:50000 8.8.8.8:443',
            'tcp ESTAB 0 0 127.0.0.1:50000 8.8.8.8:443 users:(("curl",pid=12,fd=3))',
            'tcp ESTAB 0 0 127.0.0.1:50000 127.0.0.1:443 users:(("sing-box",pid=10,fd=3))',
        ])
        self.assertEqual(families.count_rows(rows, {443}), {'inbound_or_listener': 1, 'tcp_ipv4': 1, 'udp_ipv6': 1, 'unattributed': 2, 'other': 1})

    def test_literal_family_rejects_mismatch_and_ipv4_mapping(self):
        for host, family in [('8.8.8.8', 6), ('::1', 4), ('::ffff:8.8.8.8', 6)]:
            with self.assertRaises(ValueError):
                front.destination(host, 443, family)

    def test_dns_family_filter_and_preserved_port(self):
        for family, address, kind in [(4, '8.8.8.8', 1), (6, '2606:4700:4700::1111', 4)]:
            af = socket.AF_INET if family == 4 else socket.AF_INET6
            with patch.object(socket, 'getaddrinfo', return_value=[(af, socket.SOCK_STREAM, 0, '', (address, 8443))]) as resolver:
                encoded = front.destination('fixture.test', 8443, family)
                self.assertEqual(encoded[0], kind)
                self.assertEqual(encoded[-2:], struct.pack('!H', 8443))
                self.assertEqual(resolver.call_args.args[2], af)
        with patch.object(socket, 'getaddrinfo', return_value=[]):
            with self.assertRaises(ValueError):
                front.destination('only-aaaa.test', 443, 4)

    def test_front_refuses_credentials_and_nonloopback_upstream(self):
        for proxy in ['socks5://8.8.8.8:1080', 'socks5://user:secret@127.0.0.1:1080', 'https://127.0.0.1:1080']:
            with self.assertRaises(ValueError):
                with front.literal_front(proxy, 4):
                    self.fail('invalid upstream accepted')


def pid_alive(pid):
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    # A zombie still answers kill(0); treat "exited" as gone.
    try:
        with open(f"/proc/{pid}/stat") as stat:
            return stat.read().rsplit(")", 1)[1].split()[0] != "Z"
    except OSError:
        return True


@unittest.skipUnless(os.name == "posix", "process-group cleanup is POSIX-specific")
class Cleanup(unittest.TestCase):
    def test_terminate_owned_stops_only_tracked_processes_and_closes_sockets(self):
        owned = [subprocess.Popen(["sleep", "300"]) for _ in range(2)]
        bystander = subprocess.Popen(["sleep", "300"])
        sock = socket.socket()
        try:
            lab.terminate_owned(owned, [sock])
            self.assertTrue(all(p.poll() is not None for p in owned), "a tracked process survived cleanup")
            self.assertIsNone(bystander.poll(), "cleanup killed a process the lab does not own")
            self.assertEqual(sock.fileno(), -1, "socket left open")
        finally:
            bystander.kill()
            bystander.wait()

    def injected_sigterm(self, install_handler):
        """Run a lab-shaped runner, SIGTERM it mid-run; return (exit status, child still alive)."""
        with tempfile.TemporaryDirectory() as tmp:
            pidfile = Path(tmp) / "child.pid"
            script = """
import importlib.util, subprocess, sys, time
spec = importlib.util.spec_from_file_location('lab', sys.argv[1])
lab = importlib.util.module_from_spec(spec)
spec.loader.exec_module(lab)
if sys.argv[3] == '1':
    lab.unwind_on_termination()
owned = []
try:
    owned.append(subprocess.Popen(['sleep', '300']))
    open(sys.argv[2], 'w').write(str(owned[0].pid))
    time.sleep(300)
finally:
    lab.terminate_owned(owned)
"""
            runner = subprocess.Popen([sys.executable, "-I", "-c", script, str(Path(lab.__file__)), str(pidfile), "1" if install_handler else "0"])
            child = None
            try:
                for _ in range(200):
                    if pidfile.exists() and pidfile.read_text():
                        break
                    time.sleep(0.05)
                else:
                    self.fail("the injected-failure runner never started its child")
                child = int(pidfile.read_text())
                self.assertTrue(pid_alive(child))
                runner.send_signal(signal.SIGTERM)
                status = runner.wait(timeout=15)
                for _ in range(100):
                    if not pid_alive(child):
                        break
                    time.sleep(0.05)
                return status, pid_alive(child)
            finally:
                if runner.poll() is None:
                    runner.kill()
                    runner.wait()
                if child is not None and pid_alive(child):
                    os.kill(child, signal.SIGKILL)

    def test_sigterm_mid_run_unwinds_and_reaps_children(self):
        status, orphaned = self.injected_sigterm(install_handler=True)
        self.assertEqual(status, 128 + signal.SIGTERM)
        self.assertFalse(orphaned, "SIGTERM left the lab's child process running")

    def test_control_without_the_handler_sigterm_does_orphan_the_child(self):
        # Proves the handler is what makes the test above meaningful: Python's
        # default SIGTERM disposition skips `finally`, so the child survives.
        status, orphaned = self.injected_sigterm(install_handler=False)
        self.assertEqual(status, -signal.SIGTERM)
        self.assertTrue(orphaned, "control no longer reproduces the leak; the handler test is vacuous")


if __name__ == '__main__':
    unittest.main()
