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
    def test_workspace_preview_exports_pdf(self):
        meta = Response({"id": "file-1", "name": "Sample notes", "mimeType": "application/vnd.google-apps.document",
                         "capabilities": {"canDownload": True}})
        session = Session([meta, Response(chunks=[b"%PDF-1.4"], length=8)])
        with tempfile.TemporaryDirectory() as folder, patch.object(helper.requests, "Session", return_value=session):
            path = Path(folder) / "temporary"
            self.assertEqual(helper.download_file("fixture", "file-1", path, preview=True),
                             {"saved": True, "name": "Sample notes.pdf"})
            self.assertEqual(path.read_bytes(), b"%PDF-1.4")
            self.assertTrue(session.calls[1][0].endswith("/file-1/export"))
            self.assertEqual(session.calls[1][1]["params"], {"mimeType": "application/pdf"})

    def test_download_streams_to_caller_owned_temporary_file(self):
        meta = Response({"id": "file-1", "mimeType": "application/pdf", "capabilities": {"canDownload": True}})
        session = Session([meta, Response(chunks=[b"abc", b"def"], length=6)])
        with tempfile.TemporaryDirectory() as folder, patch.object(helper.requests, "Session", return_value=session):
            path = Path(folder) / "result.pdf"
            path.write_bytes(b"old")
            temporary = Path(folder) / "temporary"
            temporary.touch()
            self.assertEqual(helper.download_file("fixture", "file-1", temporary, "key-1"), {"saved": True})
            self.assertEqual(temporary.read_bytes(), b"abcdef")
            self.assertEqual(path.read_bytes(), b"old")
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
                temporary = Path(folder) / "temporary"
                temporary.touch()
                session = Session(responses)
                with patch.object(helper.requests, "Session", return_value=session):
                    with self.assertRaises(helper.UploadError):
                        helper.download_file("fixture", "file-1", temporary)
                self.assertEqual(path.read_bytes(), b"original")
                temporary.unlink()


if __name__ == "__main__":
    unittest.main()
