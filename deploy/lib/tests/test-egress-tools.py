#!/usr/bin/env python3
import importlib.util
from pathlib import Path
import socket
import sys
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


if __name__ == '__main__':
    unittest.main()
