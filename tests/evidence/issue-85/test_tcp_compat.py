"""Isolated harness checks; no Iroh endpoints, keys or public relay traffic."""

import json
from pathlib import Path
import socket
import subprocess
import tempfile
import threading
import unittest
from unittest.mock import Mock, patch

import tcp_compat as compat


class TcpCompatTests(unittest.TestCase):
    def test_two_overlapping_channels_preserve_distinct_bytes(self):
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            listener.listen()
            listener.settimeout(5)
            record = {}
            workers = []
            server = threading.Thread(target=compat.target_channels,
                                      args=(listener, record, workers))
            server.start()
            compat.exchange_channels(listener.getsockname(), record, lambda: None)
            server.join(5)
            for worker in workers:
                worker.join(5)
                self.assertFalse(worker.is_alive())
            self.assertFalse(server.is_alive())
            self.assertNotIn("error", record)
            self.assertEqual(len(record["channels"]), 2)
            self.assertNotEqual(record["channels"][0]["request_hex"],
                                record["channels"][1]["request_hex"])
            for channel in record["channels"]:
                self.assertEqual(channel["target_received_hex"], channel["request_hex"])
                self.assertEqual(channel["client_received_hex"], channel["response_hex"])
                self.assertEqual(channel["target_round2_hex"], channel["request_hex"])
                self.assertEqual(channel["client_round2_hex"], channel["response_hex"])

    def test_shared_connection_observation_accepts_one_dial(self):
        with tempfile.TemporaryDirectory() as root:
            access = Path(root) / "access.log"
            serve = Path(root) / "serve.log"
            access.write_text("connected to serve peer (multiplexed)\n")
            serve.write_text("peer connected via relay=https://example.invalid/\n")
            observation = compat.observe_connections(access, serve, True)
            self.assertEqual(observation["access_connections"], 1)
            self.assertEqual(observation["serve_connections"], 1)

    def test_shared_connection_observation_rejects_multiple_dials(self):
        with tempfile.TemporaryDirectory() as root:
            access = Path(root) / "access.log"
            serve = Path(root) / "serve.log"
            access.write_text("connected to serve peer (multiplexed)\n" * 2)
            serve.write_text("peer connected via relay=https://example.invalid/\n" * 2)
            with self.assertRaisesRegex(RuntimeError, "connection count"):
                compat.observe_connections(access, serve, True)
            observation = compat.observe_connections(access, serve, False)
            self.assertEqual(observation["serve_connections"], 2)

    def test_authentic_serve_log_formats_and_disconnects(self):
        legacy = ("\x1b[2m2026-10-29T13:38:41.440042Z\x1b[0m "
                  "\x1b[32m INFO\x1b[0m peer connected "
                  "\x1b[3mpeer\x1b[0m\x1b[2m=\x1b[0m45260f "
                  "\x1b[3mservice\x1b[0m\x1b[2m=\x1b[0mcompat\n")
        current = ("\x1b[2m2026-10-29T13:38:21.556087Z\x1b[0m "
                   "\x1b[32m INFO\x1b[0m peer connected via relay=https://example.invalid/ "
                   "\x1b[3mpeer\x1b[0m\x1b[2m=\x1b[0m803461 "
                   "\x1b[3mservice\x1b[0m\x1b[2m=\x1b[0mcompat\n")
        disconnect = "2026-10-29T13:38:22Z INFO peer disconnected (service compat) peer=803461\n"
        with tempfile.TemporaryDirectory() as root:
            access, serve = Path(root) / "access", Path(root) / "serve"
            access.write_text("connected to serve peer (multiplexed)\n")
            for event in (legacy, current):
                with self.subTest(event=event):
                    serve.write_text(event + disconnect)
                    self.assertEqual(compat.observe_connections(access, serve, True)["serve_connections"], 1)
                    serve.write_text(event * 2 + disconnect)
                    with self.assertRaisesRegex(RuntimeError, "connection count"):
                        compat.observe_connections(access, serve, True)
                    access.write_text("connected to serve peer (per-channel)\n" * 2)
                    self.assertEqual(compat.observe_connections(access, serve, False)["serve_connections"], 2)
                    access.write_text("connected to serve peer (multiplexed)\n")

    def test_missing_or_misleading_serve_telemetry_fails_closed(self):
        with tempfile.TemporaryDirectory() as root:
            access, serve = Path(root) / "access", Path(root) / "serve"
            access.write_text("connected to serve peer (multiplexed)\n")
            for text in ("", "peer disconnected (service compat) peer=abc\n",
                         "INFO not peer connected peer=abc service=compat\n",
                         "INFO peer connectedness peer=abc service=compat\n"):
                with self.subTest(text=text):
                    serve.write_text(text)
                    with self.assertRaisesRegex(RuntimeError, "connection count"):
                        compat.observe_connections(access, serve, True)
            access.write_text("disconnected from serve peer (service compat)\n")
            serve.write_text("INFO peer connected peer=abc service=compat\n")
            with self.assertRaisesRegex(RuntimeError, "connection count"):
                compat.observe_connections(access, serve, True)

    def test_graceful_cleanup_keeps_pass(self):
        child = Mock(pid=42, returncode=0)
        child.poll.return_value = None
        record = {"outcome": "PASS", "cleanup": []}
        compat.cleanup_children([child], record)
        self.assertEqual(record["outcome"], "PASS")
        self.assertEqual(record["cleanup_errors"], [])
        child.send_signal.assert_called_once_with(compat.signal.SIGINT)
        child.wait.assert_called_once_with(timeout=15)
        child.kill.assert_not_called()

    def test_cleanup_failure_never_keeps_pass(self):
        for failure in ("nonzero", "forced", "signal", "premature", "wait", "kill"):
            with self.subTest(failure=failure):
                child = Mock(pid=42, returncode=0)
                child.poll.return_value = 0 if failure == "premature" else None
                if failure == "nonzero":
                    child.returncode = 7
                elif failure == "forced":
                    child.wait.side_effect = [subprocess.TimeoutExpired("role", 15), None]
                elif failure == "signal":
                    child.send_signal.side_effect = OSError("signal failed")
                elif failure == "wait":
                    child.wait.side_effect = OSError("wait failed")
                elif failure == "kill":
                    child.wait.side_effect = subprocess.TimeoutExpired("role", 15)
                    child.kill.side_effect = OSError("kill failed")
                record = {"outcome": "PASS", "cleanup": []}
                compat.cleanup_children([child], record)
                self.assertEqual(record["outcome"], "FAIL")
                self.assertTrue(record["cleanup_errors"])

    def test_real_child_nonzero_shutdown_forbids_pass(self):
        with tempfile.TemporaryDirectory() as root:
            ready = Path(root) / "ready"
            program = ("import signal, sys, time; from pathlib import Path; "
                       "signal.signal(signal.SIGINT, lambda *_: sys.exit(7)); "
                       "Path(sys.argv[1]).touch(); time.sleep(60)")
            child = subprocess.Popen(["uv", "run", "--no-project", "--offline",
                                      "python", "-c", program, str(ready)])
            record = {"outcome": "PASS", "cleanup": []}
            try:
                compat.wait_ready(child, ready.exists, "isolated child")
                compat.cleanup_children([child], record)
                self.assertEqual(child.returncode, 7)
                self.assertEqual(record["outcome"], "FAIL")
                self.assertFalse(record["cleanup"][0]["forced_kill"])
            finally:
                if child.poll() is None:
                    child.kill()
                child.wait(timeout=5)

    def test_cleanup_failure_makes_runner_exit_nonzero(self):
        with tempfile.TemporaryDirectory() as root:
            output = Path(root) / "new-run"
            child = Mock(pid=42, returncode=9)
            child.poll.return_value = None

            def failed_cleanup(*args):
                record = {"outcome": "PASS", "cleanup": []}
                compat.cleanup_children([child], record)
                return record

            env = {"IROH_INTEROP_OUTPUT": str(output),
                   "IROH_INTEROP_RELAY": "http://127.0.0.1:1",
                   "IROH_INTEROP_OLD_BIN": "unused", "IROH_INTEROP_OLD_SHA256": "unused",
                   "IROH_INTEROP_CURRENT_BIN": "unused",
                   "IROH_INTEROP_CURRENT_SHA256": "unused"}
            with patch.dict(compat.os.environ, env), \
                    patch.object(compat, "artifact", return_value={"version": "iroh-tunnel 0.4.0"}), \
                    patch.object(compat, "case", side_effect=failed_cleanup):
                self.assertEqual(compat.main(), 1)
            self.assertEqual(json.loads((output / "results.json").read_text())["outcome"], "FAIL")


if __name__ == "__main__":
    unittest.main()
