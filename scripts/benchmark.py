#!/usr/bin/env python3
"""Controlled SOCKS5 TCP benchmark; Python standard library only."""
import argparse
import concurrent.futures
import ipaddress
import json
import math
import socket
import socketserver
import statistics
import struct
import time


def receive(sock, count):
    data = bytearray()
    while len(data) < count:
        chunk = sock.recv(count - len(data))
        if not chunk:
            raise ConnectionError('Unexpected EOF')
        data.extend(chunk)
    return bytes(data)


class Endpoint(socketserver.BaseRequestHandler):
    def handle(self):
        sock = self.request
        sock.settimeout(30)
        sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        try:
            while True:
                command = sock.recv(1)
                if not command:
                    return
                if command == b'P':
                    payload = receive(sock, 32)
                    # Separate short writes exercise small-message forwarding.
                    sock.sendall(payload[:16])
                    sock.sendall(payload[16:])
                elif command in (b'D', b'U'):
                    remaining = struct.unpack('!Q', receive(sock, 8))[0]
                    while remaining:
                        size = min(remaining, 65536)
                        if command == b'D':
                            sock.sendall(b'x' * size)
                        else:
                            if receive(sock, size) != b'x' * size:
                                raise ValueError('Upload data mismatch')
                        remaining -= size
                    if command == b'U':
                        sock.sendall(b'K')
                else:
                    raise ValueError('Unknown benchmark command')
        except (ConnectionError, TimeoutError):
            return


def connect(args):
    sock = socket.create_connection((args.proxy, args.proxy_port), args.timeout)
    try:
        sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        sock.sendall(b'\x05\x01\x00')
        if receive(sock, 2) != b'\x05\x00':
            raise ValueError('Benchmark requires a proxy with no authentication')
        try:
            address = ipaddress.ip_address(args.target)
            encoded = bytes([1 if address.version == 4 else 4]) + address.packed
        except ValueError:
            host = args.target.encode('idna')
            encoded = b'\x03' + bytes([len(host)]) + host
        sock.sendall(b'\x05\x01\x00' + encoded + struct.pack('!H', args.target_port))
        version, reply, reserved, kind = receive(sock, 4)
        if (version, reply, reserved) != (5, 0, 0):
            raise ValueError(f'SOCKS CONNECT failed: {reply}')
        if kind == 1:
            receive(sock, 4)
        elif kind == 4:
            receive(sock, 16)
        elif kind == 3:
            receive(sock, receive(sock, 1)[0])
        else:
            raise ValueError(f'Invalid SOCKS address type: {kind}')
        receive(sock, 2)
        return sock
    except BaseException:
        sock.close()
        raise


def distribution(samples):
    ordered = sorted(samples)
    return {'median_ms': statistics.median(ordered),
            'p95_ms': ordered[math.ceil(len(ordered) * .95) - 1],
            'samples': len(ordered)}


def measure(args):
    setup = []
    for _ in range(args.connections):
        start = time.perf_counter()
        with connect(args):
            setup.append((time.perf_counter() - start) * 1000)
    latency = []
    with connect(args) as sock:
        for i in range(args.samples + 20):
            start = time.perf_counter()
            sock.sendall(b'P' + b'x' * 16)
            sock.sendall(b'x' * 16)
            if receive(sock, 32) != b'x' * 32:
                raise ValueError('Echo data mismatch')
            if i >= 20:
                latency.append((time.perf_counter() - start) * 1000)

    def transfer(command):
        # Exclude connection setup from the timed transfer.
        with connect(args) as sock:
            start = time.perf_counter()
            sock.sendall(command + struct.pack('!Q', args.bytes))
            remaining = args.bytes
            while remaining:
                size = min(remaining, 65536)
                if command == b'U':
                    sock.sendall(b'x' * size)
                elif receive(sock, size) != b'x' * size:
                    raise ValueError('Download data mismatch')
                remaining -= size
            if command == b'U' and receive(sock, 1) != b'K':
                raise ValueError('Missing upload acknowledgement')
            return args.bytes * 8 / (time.perf_counter() - start) / 1e6

    return {'connect': distribution(setup), 'round_trip': distribution(latency),
            'download_mbps': transfer(b'D'), 'upload_mbps': transfer(b'U')}


def positive(value):
    number = int(value)
    if number < 1:
        raise argparse.ArgumentTypeError('Must be positive')
    return number


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    server = commands.add_parser('serve', help='Start the controlled TCP endpoint')
    server.add_argument('--bind', default='127.0.0.1')
    server.add_argument('--port', type=int, default=9000)
    client = commands.add_parser('run', help='Measure through a SOCKS5 proxy')
    client.add_argument('--proxy', default='127.0.0.1')
    client.add_argument('--proxy-port', type=int, default=1080)
    client.add_argument('--target', default='127.0.0.1')
    client.add_argument('--target-port', type=int, default=9000)
    client.add_argument('--samples', type=positive, default=500)
    client.add_argument('--connections', type=positive, default=30)
    client.add_argument('--bytes', type=positive, default=16 * 1024 * 1024)
    client.add_argument('--clients', type=positive, default=1)
    client.add_argument('--timeout', type=positive, default=30)
    client.add_argument('--label', default='unlabelled')
    args = parser.parse_args()
    if args.command == 'serve':
        class Server(socketserver.ThreadingTCPServer):
            allow_reuse_address = True
            daemon_threads = True
            address_family = socket.AF_INET6 if ':' in args.bind else socket.AF_INET
        with Server((args.bind, args.port), Endpoint) as endpoint:
            print(f'Listening on {endpoint.server_address}', flush=True)
            endpoint.serve_forever()
    else:
        with concurrent.futures.ThreadPoolExecutor(max_workers=args.clients) as pool:
            results = list(pool.map(lambda _: measure(args), range(args.clients)))
        print(json.dumps({'parameters': vars(args), 'clients': results}, indent=2))


if __name__ == '__main__':
    main()
