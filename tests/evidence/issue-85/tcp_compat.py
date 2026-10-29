"""Actual executable TCP-COMPAT-01 runner; invoked by tests/interop_0_4.rs."""

from contextlib import ExitStack
import hashlib
import json
import os
from pathlib import Path
import platform
import signal
import socket
import subprocess
import tempfile
import threading
import time
from datetime import datetime, timezone
from urllib.parse import urlsplit

REQUEST = b"TCP-COMPAT-01 request\x00\x80\xff\n" + bytes(range(256)) * 4
RESPONSE = b"fixed target response\x00\xff\n" + bytes(range(255, -1, -1)) * 4


def now():
    return datetime.now(timezone.utc).isoformat()


def digest(data):
    return hashlib.sha256(data).hexdigest()


def receive(stream, size):
    data = bytearray()
    while len(data) < size:
        chunk = stream.recv(size - len(data))
        if not chunk:
            raise RuntimeError(f"EOF after {len(data)} of {size} bytes")
        data.extend(chunk)
    return bytes(data)


def artifact(path, expected_sha256):
    path = Path(path).resolve(strict=True)
    actual_sha256 = digest(path.read_bytes())
    if actual_sha256 != expected_sha256:
        raise ValueError(f"artifact digest mismatch: {path}")
    version = subprocess.run([str(path), "--version"], check=True,
                             capture_output=True, text=True, timeout=10)
    return {"path": str(path), "sha256": actual_sha256,
            "version": version.stdout.strip()}


def wait_ready(process, predicate, description):
    deadline = time.monotonic() + 25
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"{description}: process exited {process.returncode}")
        value = predicate()
        if value:
            return value
        time.sleep(0.1)
    raise TimeoutError(f"{description}: not ready within 25s")


def status(path):
    try:
        return json.loads(path.read_text())
    except (FileNotFoundError, json.JSONDecodeError):
        return None


def channel_records():
    return [{"channel": index,
             "request_hex": (bytes([index]) + REQUEST).hex(),
             "response_hex": (bytes([index]) + RESPONSE).hex()}
            for index in range(2)]


def target_channels(listener, record, workers):
    """Keep both upstream sockets open until clients complete round two."""
    record.setdefault("channels", channel_records())

    def handle(stream):
        channel = None
        try:
            with stream:
                stream.settimeout(30)
                data = receive(stream, len(REQUEST) + 1)
                if data[0] not in (0, 1):
                    raise RuntimeError("unknown channel marker")
                channel = record["channels"][data[0]]
                channel["target_received_hex"] = data.hex()
                if data.hex() != channel["request_hex"]:
                    raise RuntimeError("target request bytes differ")
                stream.sendall(bytes.fromhex(channel["response_hex"]))
                data = receive(stream, len(REQUEST) + 1)
                channel["target_round2_hex"] = data.hex()
                if data.hex() != channel["request_hex"]:
                    raise RuntimeError("target round two bytes differ")
                stream.sendall(bytes.fromhex(channel["response_hex"]))
        except Exception as error:
            (channel if channel is not None else record)["error"] = f"{type(error).__name__}: {error}"

    try:
        for _ in range(2):
            stream = listener.accept()[0]
            worker = threading.Thread(target=handle, args=(stream,))
            workers.append(worker)
            worker.start()
    except Exception as error:
        record["error"] = f"{type(error).__name__}: {error}"


def exchange_channels(address, record, observe):
    record.setdefault("channels", channel_records())
    with ExitStack() as stack:
        clients = []
        # First channel stays open while second transfers distinct bytes.
        for channel in record["channels"]:
            client = stack.enter_context(socket.create_connection(address, timeout=30))
            clients.append(client)
            client.settimeout(30)
            client.sendall(bytes.fromhex(channel["request_hex"]))
            data = receive(client, len(RESPONSE) + 1)
            channel["client_received_hex"] = data.hex()
            if data.hex() != channel["response_hex"]:
                raise RuntimeError("client response bytes differ")
        observe()  # Both channels live; connection count must not mean peer count.
        # Second channel closes first; first must still work independently.
        for client, channel in reversed(list(zip(clients, record["channels"]))):
            client.sendall(bytes.fromhex(channel["request_hex"]))
            data = receive(client, len(RESPONSE) + 1)
            channel["client_round2_hex"] = data.hex()
            if data.hex() != channel["response_hex"]:
                raise RuntimeError("client round two response bytes differ")
            client.close()


def observe_connections(access_log, serve_log, shared):
    access_count = access_log.read_text().count("connected to serve peer (")
    serve_count = serve_log.read_text().count("peer connected via ")
    expected = 1 if shared else 2
    if access_count != expected or serve_count != expected:
        raise RuntimeError(f"connection count: access={access_count}, serve={serve_count}, expected={expected}")
    return {"shared_required": shared, "access_connections": access_count,
            "serve_connections": serve_count,
            "basis": "connection-established log events with two overlapping TCP channels"}


def cleanup_children(children, record):
    errors = record.setdefault("cleanup_errors", [])
    for child in reversed(children):
        forced = False
        premature = child.poll() is not None
        try:
            if premature:
                errors.append(f"role {child.pid} exited before controlled shutdown")
            else:
                child.send_signal(signal.SIGINT)
            try:
                child.wait(timeout=15)
            except subprocess.TimeoutExpired:
                forced = True
                child.kill()
                child.wait(timeout=5)
            if forced or child.returncode != 0:
                errors.append(f"role {child.pid}: exit={child.returncode}, forced_kill={forced}")
        except Exception as error:
            errors.append(f"role {child.pid}: {type(error).__name__}: {error}")
            try:
                forced = True
                child.kill()
                child.wait(timeout=5)
            except Exception as kill_error:
                errors.append(f"role {child.pid} kill/wait: {kill_error}")
        record["cleanup"].append({"pid": child.pid, "exit": child.returncode,
                                  "forced_kill": forced, "premature_exit": premature})
    if errors:
        record["outcome"] = "FAIL"


def case(output, old, current, relay, serve_kind, access_kind, multiplex):
    name = f"{access_kind}-access_to_{serve_kind}-serve"
    if access_kind == "old":
        name += f"_multiplex-{str(multiplex).lower()}"
    record = {"case": "TCP-COMPAT-01", "variant": name, "started_at": now(),
              "serve_kind": serve_kind, "access_kind": access_kind,
              "old_access_multiplex": multiplex if access_kind == "old" else None,
              "commands": [], "channels": channel_records(), "cleanup": []}
    children = []
    logs = []
    worker = None
    target_workers = []
    phase = "serve-startup"
    private = tempfile.TemporaryDirectory(prefix="iroh-85-")
    listener = socket.socket()
    listener.bind(("127.0.0.1", 0))
    listener.listen()
    listener.settimeout(35)
    target_port = listener.getsockname()[1]
    record["target_address"] = f"127.0.0.1:{target_port}"

    try:
        root = Path(private.name)

        def spawn(kind, role, args):
            home = root / role
            home.mkdir()
            state = home / "state"
            state.mkdir()
            binary = old if kind == "old" else current
            command = [binary["path"], "--color", "never", role] + args
            env = {"HOME": str(home), "XDG_CONFIG_HOME": str(home),
                   "IROH_TUNNEL_TEST_CONFIG_DIR": str(home / "config"),
                   "IROH_TUNNEL_STATE_DIR": str(state),
                   "RUST_LOG": "iroh_tunnel=info"}
            log_path = output / f"{name}.{role}.log"
            log = log_path.open("w")
            logs.append(log)
            process = subprocess.Popen(command, env={**os.environ, **env},
                                       stdout=log, stderr=subprocess.STDOUT)
            children.append(process)
            record["commands"].append({"argv": command, "env": env,
                                       "pid": process.pid, "log": log_path.name})
            return process, state, log_path

        if serve_kind == "old":
            config = root / "serve.toml"
            config.write_text(f'[node]\nrelay_urls = ["{relay}"]\n\n'
                              '[[services]]\nname = "compat"\nprotocol = "tcp"\n'
                              f'host = "127.0.0.1"\nport = {target_port}\n')
            config.chmod(0o600)
            args = ["run", "--config", str(config)]
        else:
            args = ["--name", "compat", "--target", f"127.0.0.1:{target_port}",
                    "--relay-url", relay]
        serve, serve_state, serve_log = spawn(serve_kind, "serve", args)
        serve_status = wait_ready(serve, lambda: status(serve_state / "serve-status.json"),
                                  "Serve status")
        node = serve_status["node_id"]
        record["serve_node_id"] = node
        # Reserve then release; listener ownership is verified from role log, not a TCP probe.
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            access_port = reservation.getsockname()[1]
        record["client_address"] = f"127.0.0.1:{access_port}"
        if access_kind == "old":
            config = root / "access.toml"
            config.write_text(f'[node]\nrelay_urls = ["{relay}"]\n\n'
                              '[[services]]\nname = "compat"\nprotocol = "tcp"\n'
                              f'node_id = "{node}"\nhost = "127.0.0.1"\n'
                              f'port = {access_port}\nmultiplex = {str(multiplex).lower()}\n')
            config.chmod(0o600)
            args = ["run", "--config", str(config)]
        else:
            args = ["--name", "compat", "--listen", f"127.0.0.1:{access_port}",
                    "--node-id", node, "--relay-url", relay]
        phase = "access-startup"
        access, access_state, log = spawn(access_kind, "access", args)
        wait_ready(access, lambda: any(
            "listening for local clients" in line and f"127.0.0.1:{access_port}" in line
            for line in log.read_text().splitlines()), "Access listener")
        phase = "tcp-exchange"
        worker = threading.Thread(target=target_channels,
                                  args=(listener, record, target_workers), name=name)
        worker.start()

        def observe():
            record["connection_observation"] = observe_connections(
                log, serve_log, access_kind != "old" or multiplex)

        exchange_channels(("127.0.0.1", access_port), record, observe)
        worker.join(timeout=36)
        for target_worker in target_workers:
            target_worker.join(timeout=36)
        if worker.is_alive() or any(item.is_alive() for item in target_workers):
            raise RuntimeError("target threads did not finish")
        if "error" in record or any("error" in item for item in record["channels"]):
            raise RuntimeError("target channel failed; inspect channel transcripts")
        record["access_status"] = status(access_state / "access-status.json")
        record["serve_status"] = status(serve_state / "serve-status.json")
        record["outcome"] = "PASS"
    except Exception as error:
        record["outcome"] = "FAIL" if phase == "tcp-exchange" else "BLOCKED"
        record["phase"] = phase
        record["error"] = f"{type(error).__name__}: {error}"
    finally:
        # PASS requires graceful exit 0 for every owned role, never forced kill.
        cleanup_children(children, record)
        listener.close()
        if worker is not None:
            worker.join(timeout=36)
            for target_worker in target_workers:
                target_worker.join(timeout=36)
            if worker.is_alive() or any(item.is_alive() for item in target_workers):
                record["outcome"] = "FAIL"
                record["error"] = "target thread did not terminate"
        for log in logs:
            log.close()
        try:
            private.cleanup()
        except Exception as error:
            record["outcome"] = "FAIL"
            record.setdefault("cleanup_errors", []).append(f"temporary directory cleanup: {error}")
    record["finished_at"] = now()
    print(f"{name}: {record['outcome']} {record.get('error', '')}", flush=True)
    return record


def main():
    output = Path(os.environ["IROH_INTEROP_OUTPUT"]).resolve()
    output.mkdir(parents=True, exist_ok=False)
    report = {"case": "TCP-COMPAT-01", "started_at": now(), "platform": platform.platform(),
              "machine": platform.machine(), "user_id": os.getuid(),
              "harness_sha256": digest(Path(__file__).read_bytes()), "variants": []}
    try:
        relay = os.environ["IROH_INTEROP_RELAY"]
        url = urlsplit(relay)
        if url.username or url.password or url.query or url.fragment:
            raise ValueError("relay URL must not contain credentials, query or fragment")
        if url.scheme != "https" and not (url.scheme == "http" and url.hostname == "127.0.0.1"):
            raise ValueError("relay must be HTTPS, or HTTP on loopback for startup diagnostics")
        # Pin independently verified production artifacts, never Cargo's feature-unified binary.
        old = artifact(os.environ["IROH_INTEROP_OLD_BIN"], os.environ["IROH_INTEROP_OLD_SHA256"])
        current = artifact(os.environ["IROH_INTEROP_CURRENT_BIN"],
                           os.environ["IROH_INTEROP_CURRENT_SHA256"])
        if not old["version"].startswith("iroh-tunnel 0.4."):
            raise ValueError("old executable must report actual 0.4 version")
        report.update(old=old, current=current, relay=relay,
                      request_hex=REQUEST.hex(), response_hex=RESPONSE.hex())
        for serve, access, mux in [("old", "old", True), ("current", "current", True),
                                   ("current", "old", True), ("old", "current", True),
                                   ("current", "old", False), ("old", "old", False)]:
            report["variants"].append(case(output, old, current, relay, serve, access, mux))
        report["outcome"] = "PASS" if all(
            item["outcome"] == "PASS" for item in report["variants"]) else "FAIL"
    except Exception as error:
        report["outcome"] = "BLOCKED"
        report["error"] = f"{type(error).__name__}: {error}"
    report["finished_at"] = now()
    (output / "results.json").write_text(json.dumps(report, indent=2) + "\n")
    return 0 if report["outcome"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
