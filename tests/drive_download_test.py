import importlib.util
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("drive_helper", Path(__file__).resolve().parents[1] / "desktop/drive_upload.py")
helper = importlib.util.module_from_spec(spec)
spec.loader.exec_module(helper)


class Response:
    def __init__(self, body=None, chunks=(), status=200, length=None):
        self.body = body
        self.chunks = chunks
        self.status_code = status
        self.headers = {} if length is None else {"Content-Length": str(length)}

    def json(self):
        return self.body

    def iter_content(self, chunk_size):
        yield from self.chunks

    def __enter__(self):
        return self

    def __exit__(self, *args):
        pass


class Session:
    def __init__(self, responses):
        self.responses = iter(responses)
        self.headers = {}
        self.calls = []

    def __enter__(self):
        return self

    def __exit__(self, *args):
        pass

    def get(self, url, **kwargs):
        self.calls.append((url, kwargs))
        return next(self.responses)


class DriveDownloadTests(unittest.TestCase):
    def test_download_replaces_destination_only_after_complete_response(self):
        meta = Response({"id": "file-1", "mimeType": "application/pdf", "capabilities": {"canDownload": True}})
        session = Session([meta, Response(chunks=[b"abc", b"def"], length=6)])
        with tempfile.TemporaryDirectory() as folder, patch.object(helper.requests, "Session", return_value=session):
            path = Path(folder) / "result.pdf"
            path.write_bytes(b"old")
            self.assertEqual(helper.download_file("fixture", "file-1", path, "key-1"), {"saved": True})
            self.assertEqual(path.read_bytes(), b"abcdef")
            self.assertEqual(session.headers["X-Goog-Drive-Resource-Keys"], "file-1/key-1")
            self.assertEqual(session.calls[1][1]["params"]["alt"], "media")

    def test_denied_workspace_and_incomplete_results_preserve_existing_file(self):
        cases = [
            [Response({"id": "file-1", "mimeType": "application/vnd.google-apps.document", "capabilities": {"canDownload": True}})],
            [Response({"id": "file-1", "mimeType": "application/pdf", "capabilities": {"canDownload": False}})],
            [Response({"id": "file-1", "mimeType": "application/pdf", "capabilities": {"canDownload": True}}), Response(chunks=[b"partial"], length=10)],
        ]
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "result.pdf"
            for responses in cases:
                path.write_bytes(b"original")
                session = Session(responses)
                with patch.object(helper.requests, "Session", return_value=session):
                    with self.assertRaises(helper.UploadError):
                        helper.download_file("fixture", "file-1", path)
                self.assertEqual(path.read_bytes(), b"original")
                self.assertEqual(list(Path(folder).iterdir()), [path])


if __name__ == "__main__":
    unittest.main()
