# publish-video Plugin Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract MyTV's `vod_upload.py` into a standalone, MyTV-independent, agent-invocable "publish a video to a public URL" capability, packaged as a distributable Claude Code plugin (`publish-video`).

**Architecture:** One portable Python engine (`publish_video.py`) with a pure-helper core (source classification, input resolution, playability gate, result/envelope builders) wrapping thin I/O functions (yt-dlp, ffprobe/ffmpeg, urllib download, boto3 upload, MyTV API). It is wrapped by a `SKILL.md` (agent interface) and shipped as a single-repo plugin + marketplace, built and validated locally before publishing to GitHub.

**Tech Stack:** Python 3 (stdlib + boto3), `unittest`, yt-dlp, ffmpeg/ffprobe, Claude Code plugin/skill format.

---

## Reference: seed file

The engine is seeded from `vod_upload.py` on MyTV's `feat/vod-upload-tool` branch. Retrieve it with `git show` (Task 1). Its existing I/O wrappers (`download_and_mux`, `probe_duration`, `fetch_title`, `upload_to_bucket`, `register_item`, `require_env`, `require_tool`) and pure helpers (`is_url`, `sanitize_filename`, `object_key`, `build_ytdlp_cmd`, `build_register_url`, `build_payload`, `public_url`) are reused; `main()` is fully rewritten.

## File structure

```
~/Workspace/playground/publish-video-plugin/        # new git repo (source of truth)
  .claude-plugin/
    plugin.json                                      # Task 10
    marketplace.json                                 # Task 10
  skills/publish-video/
    SKILL.md                                         # Task 9
    REFERENCE.md                                     # Task 9
    scripts/
      publish_video.py                               # Tasks 1–8 (the engine)
      test_publish_video.py                          # Tasks 1–8 (unittest)
  README.md                                          # Task 10
```

**Paths below are absolute.** `PLUGIN=~/Workspace/playground/publish-video-plugin`, `MYTV=~/Workspace/playground/MyTV`, `SCRIPTS=$PLUGIN/skills/publish-video/scripts`.

**Test command (used throughout):** `cd $SCRIPTS && python3 -m unittest test_publish_video -v`

---

### Task 1: Scaffold the plugin repo and seed the engine

**Files:**
- Create: `$PLUGIN/skills/publish-video/scripts/publish_video.py` (from seed)
- Create: `$PLUGIN/skills/publish-video/scripts/test_publish_video.py` (from seed)

- [ ] **Step 1: Create the directory tree and init git**

```bash
mkdir -p ~/Workspace/playground/publish-video-plugin/.claude-plugin
mkdir -p ~/Workspace/playground/publish-video-plugin/skills/publish-video/scripts
cd ~/Workspace/playground/publish-video-plugin && git init
```

- [ ] **Step 2: Seed the engine and tests from the MyTV feat branch**

```bash
cd ~/Workspace/playground/publish-video-plugin
git -C ~/Workspace/playground/MyTV show feat/vod-upload-tool:scripts/vod_upload.py \
  > skills/publish-video/scripts/publish_video.py
git -C ~/Workspace/playground/MyTV show feat/vod-upload-tool:scripts/test_vod_upload.py \
  > skills/publish-video/scripts/test_publish_video.py
```

- [ ] **Step 3: Fix the test import to the new module name**

In `skills/publish-video/scripts/test_publish_video.py`, change the import line:

```python
import publish_video as v
```

(was `import vod_upload as v`)

- [ ] **Step 4: Run the seeded tests — verify green**

Run: `cd ~/Workspace/playground/publish-video-plugin/skills/publish-video/scripts && python3 -m unittest test_publish_video -v`
Expected: 8 tests, all `ok` (the seeded pure-helper suite passes unchanged).

- [ ] **Step 5: Commit**

```bash
cd ~/Workspace/playground/publish-video-plugin
git add -A
git commit -m "chore: scaffold publish-video plugin, seed engine from vod_upload.py"
```

---

### Task 2: Convert per-item I/O failures to a raised exception

Batch processing must not abort on one bad item. The seed's I/O wrappers call `sys.exit` on failure, which would kill the whole run. Introduce `PublishError` and raise it from the **per-item** wrappers (`probe_duration`, `download_and_mux`, `upload_to_bucket`, `register_item`). The **config** checks (`require_env`, `require_tool`) keep `sys.exit(2)` — they run once, up front.

**Files:**
- Modify: `$SCRIPTS/publish_video.py`
- Test: `$SCRIPTS/test_publish_video.py`

- [ ] **Step 1: Write the failing test**

Add to `test_publish_video.py`:

```python
class Errors(unittest.TestCase):
    def test_publisherror_is_exception(self):
        self.assertTrue(issubclass(v.PublishError, Exception))

    def test_publisherror_carries_message(self):
        err = v.PublishError("boom")
        self.assertEqual(str(err), "boom")
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd $SCRIPTS && python3 -m unittest test_publish_video.Errors -v`
Expected: FAIL — `AttributeError: module 'publish_video' has no attribute 'PublishError'`.

- [ ] **Step 3: Add the exception and convert the per-item wrappers**

Near the top of `publish_video.py` (after the imports / `AVC1_FORMAT`):

```python
class PublishError(Exception):
    """A per-item failure; caught by the batch loop so other items continue."""


def die(msg: str):
    """Config/usage error: print to stderr and exit 2."""
    print(msg, file=sys.stderr)
    sys.exit(2)
```

Convert the two **config** checks to exit code 2 via `die` (they run once, up front — not per item):

```python
def require_env(*names):
    missing = [n for n in names if not os.environ.get(n)]
    if missing:
        die("error: missing required env vars: " + ", ".join(missing))


def require_tool(name: str):
    if shutil.which(name) is None:
        die(f"error: required tool not found on PATH: {name}")
```

Replace the body of `probe_duration` so it raises instead of exiting:

```python
def probe_duration(path: str) -> int:
    out = subprocess.run(
        [
            "ffprobe", "-v", "error",
            "-show_entries", "format=duration",
            "-of", "default=nokey=1:noprint_wrappers=1",
            path,
        ],
        capture_output=True,
        text=True,
    )
    if out.returncode != 0:
        raise PublishError(f"ffprobe failed: {out.stderr.strip()}")
    try:
        return round(float(out.stdout.strip()))
    except ValueError:
        raise PublishError(f"could not parse ffprobe duration: {out.stdout!r}")
```

In `download_and_mux`, replace the failure line:

```python
    if subprocess.run(cmd).returncode != 0:
        raise PublishError("yt-dlp download/mux failed (see output above)")
```

In `upload_to_bucket`, replace the boto3 import guard:

```python
    try:
        import boto3
    except ImportError:
        raise PublishError("boto3 is required for upload (pip install boto3)")
```

In `register_item`, replace both error branches:

```python
    except urllib.error.HTTPError as e:
        raise PublishError(f"MyTV API returned {e.code}: {e.read().decode()[:300]}")
    except urllib.error.URLError as e:
        raise PublishError(f"could not reach {url}: {e.reason}")
```

- [ ] **Step 4: Run tests — verify pass**

Run: `cd $SCRIPTS && python3 -m unittest test_publish_video -v`
Expected: all tests `ok` (8 seeded + 2 new).

- [ ] **Step 5: Commit**

```bash
cd $PLUGIN && git add -A
git commit -m "refactor: raise PublishError on per-item failures (batch resilience)"
```

---

### Task 3: Source classification

Classify each input string into one of `directory`, `local_file`, `direct_url`, `ytdlp_url`. `os.path.isdir`/`os.path.isfile` are injected so the logic is testable without a real filesystem.

**Files:**
- Modify: `$SCRIPTS/publish_video.py`
- Test: `$SCRIPTS/test_publish_video.py`

- [ ] **Step 1: Write the failing test**

```python
class Classify(unittest.TestCase):
    def test_has_media_ext(self):
        self.assertTrue(v.has_media_ext("https://x/y.MP4?token=1"))
        self.assertTrue(v.has_media_ext("https://x/y.webm"))
        self.assertFalse(v.has_media_ext("https://x/watch?v=abc"))
        self.assertFalse(v.has_media_ext("https://x/y.m3u8"))

    def test_is_video_file(self):
        self.assertTrue(v.is_video_file("Ep1.mkv"))
        self.assertTrue(v.is_video_file("a.MP4"))
        self.assertFalse(v.is_video_file("notes.txt"))

    def test_classify_local(self):
        isdir = lambda p: p == "/movies"
        isfile = lambda p: p == "/movies/a.mp4"
        self.assertEqual(v.classify_source("/movies", isdir, isfile), "directory")
        self.assertEqual(v.classify_source("/movies/a.mp4", isdir, isfile), "local_file")

    def test_classify_urls(self):
        no = lambda p: False
        self.assertEqual(v.classify_source("https://x/y.mp4", no, no), "direct_url")
        self.assertEqual(v.classify_source("https://youtu.be/abc", no, no), "ytdlp_url")

    def test_classify_unknown_raises(self):
        no = lambda p: False
        with self.assertRaises(ValueError):
            v.classify_source("./missing.mp4", no, no)
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd $SCRIPTS && python3 -m unittest test_publish_video.Classify -v`
Expected: FAIL — `module 'publish_video' has no attribute 'has_media_ext'`.

- [ ] **Step 3: Implement the classifier**

Add after the existing `is_url`:

```python
MEDIA_EXTS = (".mp4", ".webm", ".mov", ".m4v")
VIDEO_FILE_EXTS = (".mp4", ".webm", ".mov", ".m4v", ".mkv", ".avi")


def has_media_ext(url: str, exts=MEDIA_EXTS) -> bool:
    path = url.split("?", 1)[0].split("#", 1)[0].lower()
    return path.endswith(exts)


def is_video_file(name: str, exts=VIDEO_FILE_EXTS) -> bool:
    return name.lower().endswith(exts)


def classify_source(source: str, isdir=os.path.isdir, isfile=os.path.isfile) -> str:
    if isdir(source):
        return "directory"
    if isfile(source):
        return "local_file"
    if is_url(source):
        return "direct_url" if has_media_ext(source) else "ytdlp_url"
    raise ValueError(f"not a file, directory, or URL: {source}")
```

- [ ] **Step 4: Run tests — verify pass**

Run: `cd $SCRIPTS && python3 -m unittest test_publish_video.Classify -v`
Expected: all `ok`.

- [ ] **Step 5: Commit**

```bash
cd $PLUGIN && git add -A
git commit -m "feat: source classification (directory/local_file/direct_url/ytdlp_url)"
```

---

### Task 4: Input resolution, directory expansion, required tools

Flatten positional sources + `--from-file` lines into an ordered **job list** of `(source, type)` tuples; directories expand to one `local_file` job per contained video. Compute which external tools a run actually needs.

**Files:**
- Modify: `$SCRIPTS/publish_video.py`
- Test: `$SCRIPTS/test_publish_video.py`

- [ ] **Step 1: Write the failing test**

```python
class Resolve(unittest.TestCase):
    def test_parse_source_list(self):
        text = "https://a/x.mp4\n# comment\n\n  ./b.mp4  \n"
        self.assertEqual(v.parse_source_list(text), ["https://a/x.mp4", "./b.mp4"])

    def test_expand_directory(self):
        listing = {"/m": ["a.mp4", "b.txt", "c.mkv"]}
        walk = lambda p: [(p, [], listing[p])]
        got = v.expand_directory("/m", recursive=False, walk_fn=walk)
        self.assertEqual(got, ["/m/a.mp4", "/m/c.mkv"])

    def test_resolve_jobs_expands_dir(self):
        classify = lambda s, *_: {"/m": "directory", "/m/a.mp4": "local_file",
                                  "https://x/y.mp4": "direct_url"}[s]
        walk = lambda p: [("/m", [], ["a.mp4"])]
        jobs = v.resolve_jobs(["/m", "https://x/y.mp4"], recursive=False,
                              classify_fn=classify, walk_fn=walk)
        self.assertEqual(jobs, [("/m/a.mp4", "local_file"),
                                ("https://x/y.mp4", "direct_url")])

    def test_required_tools(self):
        jobs = [("u", "ytdlp_url"), ("f", "local_file")]
        self.assertEqual(v.required_tools(jobs, transcode=False), {"ffprobe", "yt-dlp"})
        self.assertEqual(v.required_tools([("f", "local_file")], transcode=True),
                         {"ffprobe", "ffmpeg"})
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd $SCRIPTS && python3 -m unittest test_publish_video.Resolve -v`
Expected: FAIL — `module 'publish_video' has no attribute 'parse_source_list'`.

- [ ] **Step 3: Implement resolution and tooling**

```python
def parse_source_list(text: str) -> list:
    out = []
    for line in text.splitlines():
        line = line.strip()
        if line and not line.startswith("#"):
            out.append(line)
    return out


def expand_directory(path: str, recursive: bool, walk_fn=os.walk) -> list:
    files = []
    for root, _dirs, names in walk_fn(path):
        for name in sorted(names):
            if is_video_file(name):
                files.append(os.path.join(root, name))
        if not recursive:
            break
    return files


def resolve_jobs(sources, recursive, classify_fn=classify_source, walk_fn=os.walk) -> list:
    jobs = []
    for source in sources:
        stype = classify_fn(source)
        if stype == "directory":
            for f in expand_directory(source, recursive, walk_fn):
                jobs.append((f, "local_file"))
        else:
            jobs.append((source, stype))
    return jobs


def required_tools(jobs, transcode: bool) -> set:
    tools = {"ffprobe"}
    if any(t == "ytdlp_url" for _, t in jobs):
        tools.add("yt-dlp")
    if transcode:
        tools.add("ffmpeg")
    return tools
```

- [ ] **Step 4: Run tests — verify pass**

Run: `cd $SCRIPTS && python3 -m unittest test_publish_video.Resolve -v`
Expected: all `ok`.

- [ ] **Step 5: Commit**

```bash
cd $PLUGIN && git add -A
git commit -m "feat: resolve sources to job list (dir expansion, from-file, required tools)"
```

---

### Task 5: Acquisition — content type, direct download, transcode command, dispatch

**Files:**
- Modify: `$SCRIPTS/publish_video.py`
- Test: `$SCRIPTS/test_publish_video.py`

- [ ] **Step 1: Write the failing test**

```python
class Acquire(unittest.TestCase):
    def test_content_type_for(self):
        self.assertEqual(v.content_type_for("/x/a.mp4"), "video/mp4")
        self.assertEqual(v.content_type_for("/x/a.webm"), "video/webm")
        self.assertEqual(v.content_type_for("/x/a.mov"), "video/quicktime")
        self.assertEqual(v.content_type_for("/x/a.unknown"), "video/mp4")

    def test_build_ffmpeg_transcode_cmd(self):
        cmd = v.build_ffmpeg_transcode_cmd("/in.mkv", "/out.mp4")
        self.assertEqual(cmd[0], "ffmpeg")
        self.assertIn("libx264", cmd)
        self.assertIn("aac", cmd)
        self.assertEqual(cmd[-1], "/out.mp4")
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd $SCRIPTS && python3 -m unittest test_publish_video.Acquire -v`
Expected: FAIL — `module 'publish_video' has no attribute 'content_type_for'`.

- [ ] **Step 3: Implement content type, transcode cmd, direct download, dispatch**

```python
CONTENT_TYPES = {
    "mp4": "video/mp4", "m4v": "video/x-m4v",
    "webm": "video/webm", "mov": "video/quicktime",
}


def content_type_for(path: str) -> str:
    ext = os.path.splitext(path)[1].lstrip(".").lower()
    return CONTENT_TYPES.get(ext, "video/mp4")


def build_ffmpeg_transcode_cmd(in_path: str, out_path: str) -> list:
    return [
        "ffmpeg", "-y", "-i", in_path,
        "-c:v", "libx264", "-c:a", "aac",
        "-movflags", "+faststart", out_path,
    ]


def download_direct(url: str, out_path: str):
    try:
        with urllib.request.urlopen(url) as resp, open(out_path, "wb") as f:
            shutil.copyfileobj(resp, f)
    except (urllib.error.URLError, OSError) as e:
        raise PublishError(f"direct download failed for {url}: {e}")


def transcode_to_h264(in_path: str, out_path: str):
    if subprocess.run(build_ffmpeg_transcode_cmd(in_path, out_path)).returncode != 0:
        raise PublishError("ffmpeg transcode failed (see output above)")


def acquire(source: str, stype: str, workdir: str, cookies, format_sort: str) -> str:
    if stype == "ytdlp_url":
        out = os.path.join(workdir, "video.mp4")
        download_and_mux(source, out, cookies, format_sort)
        return out
    if stype == "direct_url":
        name = sanitize_filename(os.path.basename(source.split("?", 1)[0])) or "video.mp4"
        out = os.path.join(workdir, name)
        download_direct(source, out)
        return out
    if stype == "local_file":
        return source
    raise PublishError(f"cannot acquire source type: {stype}")
```

- [ ] **Step 4: Run tests — verify pass**

Run: `cd $SCRIPTS && python3 -m unittest test_publish_video.Acquire -v`
Expected: all `ok`.

- [ ] **Step 5: Commit**

```bash
cd $PLUGIN && git add -A
git commit -m "feat: acquisition (content type, direct download, transcode cmd, dispatch)"
```

---

### Task 6: Playability gate

`is_browser_playable` is the pure decision; `probe_streams` reads codecs; `ensure_playable` ties them together with the warn-default / `--transcode` policy.

**Files:**
- Modify: `$SCRIPTS/publish_video.py`
- Test: `$SCRIPTS/test_publish_video.py`

- [ ] **Step 1: Write the failing test**

```python
class Playable(unittest.TestCase):
    def test_playable_true(self):
        self.assertTrue(v.is_browser_playable("mp4", "h264", "aac"))
        self.assertTrue(v.is_browser_playable("mp4", "h264", ""))   # no audio
        self.assertTrue(v.is_browser_playable("mp4", "h264", None))

    def test_playable_false(self):
        self.assertFalse(v.is_browser_playable("webm", "vp9", "opus"))
        self.assertFalse(v.is_browser_playable("mp4", "av1", "aac"))
        self.assertFalse(v.is_browser_playable("mkv", "h264", "aac"))
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd $SCRIPTS && python3 -m unittest test_publish_video.Playable -v`
Expected: FAIL — `module 'publish_video' has no attribute 'is_browser_playable'`.

- [ ] **Step 3: Implement the gate**

```python
def is_browser_playable(container: str, vcodec: str, acodec) -> bool:
    return container == "mp4" and vcodec == "h264" and (acodec in ("aac", "", None))


def probe_streams(path: str):
    def codec(kind: str) -> str:
        out = subprocess.run(
            ["ffprobe", "-v", "error", "-select_streams", kind,
             "-show_entries", "stream=codec_name",
             "-of", "default=nokey=1:noprint_wrappers=1", path],
            capture_output=True, text=True,
        )
        return out.stdout.strip()
    return codec("v:0"), codec("a:0")


def ensure_playable(path: str, transcode: bool, workdir: str):
    container = os.path.splitext(path)[1].lstrip(".").lower()
    vcodec, acodec = probe_streams(path)
    if is_browser_playable(container, vcodec, acodec):
        return path, True, False
    if transcode:
        out = os.path.join(workdir, "transcoded.mp4")
        transcode_to_h264(path, out)
        return out, False, True
    print(
        f"warning: {os.path.basename(path)} is "
        f"{container}/{vcodec or '?'}/{acodec or 'no-audio'}; may not play in all browsers",
        file=sys.stderr,
    )
    return path, False, False
```

- [ ] **Step 4: Run tests — verify pass**

Run: `cd $SCRIPTS && python3 -m unittest test_publish_video.Playable -v`
Expected: all `ok`.

- [ ] **Step 5: Commit**

```bash
cd $PLUGIN && git add -A
git commit -m "feat: playability gate (warn default, --transcode opt-in)"
```

---

### Task 7: Results, envelope, exit codes, title derivation

**Files:**
- Modify: `$SCRIPTS/publish_video.py`
- Test: `$SCRIPTS/test_publish_video.py`

- [ ] **Step 1: Write the failing test**

```python
class Results(unittest.TestCase):
    def test_build_result(self):
        r = v.build_result("src", "local_file", "T", "https://b/k.mp4", "k.mp4", 12, True, False)
        self.assertEqual(r["public_url"], "https://b/k.mp4")
        self.assertEqual(r["duration_secs"], 12)
        self.assertTrue(r["passthrough"])
        self.assertNotIn("error", r)

    def test_error_result(self):
        r = v.error_result("src", "ytdlp_url", "boom")
        self.assertEqual(r["error"], "boom")
        self.assertNotIn("public_url", r)

    def test_envelope_and_exit(self):
        ok = v.build_result("s", "local_file", "T", "u", "k", 1, True, False)
        bad = v.error_result("s2", "ytdlp_url", "x")
        env = v.build_envelope([ok, bad])
        self.assertEqual(env["ok"], 1)
        self.assertEqual(env["failed"], 1)
        self.assertEqual(env["results"], [ok, bad])
        self.assertEqual(v.exit_code_for([ok, bad]), 1)
        self.assertEqual(v.exit_code_for([ok]), 0)

    def test_derive_title_dry_run(self):
        self.assertEqual(v.derive_title("/x/My Clip.mkv", "local_file", None,
                                        cookies=None, dry_run=True), "My Clip")
        self.assertEqual(v.derive_title("https://x/y.mp4", "direct_url", None,
                                        cookies=None, dry_run=True), "y")
        self.assertEqual(v.derive_title("https://x/y.mp4", "direct_url", "Override",
                                        cookies=None, dry_run=True), "Override")
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd $SCRIPTS && python3 -m unittest test_publish_video.Results -v`
Expected: FAIL — `module 'publish_video' has no attribute 'build_result'`.

- [ ] **Step 3: Implement builders, envelope, exit code, title**

```python
def build_result(source, stype, title, public, key, duration, passthrough, transcoded) -> dict:
    return {
        "source": source, "type": stype, "title": title,
        "public_url": public, "object_key": key, "duration_secs": duration,
        "passthrough": passthrough, "transcoded": transcoded,
    }


def error_result(source, stype, message) -> dict:
    return {"source": source, "type": stype, "error": message}


def build_envelope(results) -> dict:
    failed = sum(1 for r in results if "error" in r)
    return {"ok": len(results) - failed, "failed": failed, "results": results}


def exit_code_for(results) -> int:
    return 1 if any("error" in r for r in results) else 0


def derive_title(source, stype, override, cookies, dry_run, final_path=None) -> str:
    if override:
        return override
    if stype == "local_file":
        return os.path.splitext(os.path.basename(source))[0]
    if stype == "direct_url":
        base = os.path.basename(source.split("?", 1)[0])
        return os.path.splitext(base)[0] or "Untitled"
    # ytdlp_url
    if dry_run:
        return "Untitled"
    return fetch_title(source, cookies) or "Untitled"
```

- [ ] **Step 4: Run tests — verify pass**

Run: `cd $SCRIPTS && python3 -m unittest test_publish_video.Results -v`
Expected: all `ok`.

- [ ] **Step 5: Commit**

```bash
cd $PLUGIN && git add -A
git commit -m "feat: result/envelope/exit-code builders + title derivation"
```

---

### Task 8: Orchestration — `process_job` and the new `main()`

Wire everything: new argparse surface, neutral env (`PUBLISH_VIDEO_S3_*`), fail-fast config checks, dry-run, the batch loop, sinks, and exit codes. This replaces the seed's `main()` and the old `is_url`-only branching.

**Files:**
- Modify: `$SCRIPTS/publish_video.py`
- Test: `$SCRIPTS/test_publish_video.py` (one pure test) + manual dry-run runs

- [ ] **Step 1: Write the failing test (dry-run planning is pure)**

```python
class DryRunPlan(unittest.TestCase):
    def test_plan_job_local(self):
        plan = v.plan_job("/x/My Clip.mp4", "local_file", key_prefix="video",
                          public_base="https://b", title_override=None, transcode=False,
                          uid="ID")
        self.assertEqual(plan["title"], "My Clip")
        self.assertEqual(plan["object_key"], "video/ID-My_Clip.mp4")
        self.assertEqual(plan["public_url"], "https://b/video/ID-My_Clip.mp4")
        self.assertTrue(plan["dry_run"])
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd $SCRIPTS && python3 -m unittest test_publish_video.DryRunPlan -v`
Expected: FAIL — `module 'publish_video' has no attribute 'plan_job'`.

- [ ] **Step 3: Implement `plan_job`, `process_job`, and rewrite `main()`**

Add `plan_job` (pure planning for dry-run) and `process_job` (real work), then replace the entire `main()` function:

```python
def plan_job(source, stype, key_prefix, public_base, title_override, transcode, uid) -> dict:
    title = derive_title(source, stype, title_override, cookies=None, dry_run=True)
    ext = "mp4"
    if stype in ("direct_url", "local_file"):
        ext = os.path.splitext(source.split("?", 1)[0])[1].lstrip(".").lower() or "mp4"
        if transcode and ext != "mp4":
            ext = "mp4"
    key = object_key(key_prefix, sanitize_filename(title) + "." + ext, uid)
    return {
        "source": source, "type": stype, "title": title,
        "object_key": key, "public_url": public_url(public_base, key),
        "dry_run": True,
    }


def process_job(source, stype, args, endpoint, bucket, public_base) -> dict:
    workdir = tempfile.mkdtemp(prefix="publish_video_")
    try:
        acquired = acquire(source, stype, workdir, args.cookies, args.format_sort)
        final_path, passthrough, transcoded = ensure_playable(acquired, args.transcode, workdir)
        duration = probe_duration(final_path)
        title = derive_title(source, stype, args.title, args.cookies, dry_run=False)
        ext = os.path.splitext(final_path)[1].lstrip(".").lower() or "mp4"
        key = object_key(args.key_prefix, sanitize_filename(title) + "." + ext, uuid.uuid4().hex)
        upload_to_bucket(final_path, endpoint, bucket, key, content_type_for(final_path))
        return build_result(source, stype, title, public_url(public_base, key),
                            key, duration, passthrough, transcoded)
    finally:
        shutil.rmtree(workdir, ignore_errors=True)


def main():
    p = argparse.ArgumentParser(
        description="Publish a video (file, URL, yt-dlp site, or folder) to a public URL."
    )
    p.add_argument("sources", nargs="*", help="yt-dlp URL | direct media URL | local file | local directory")
    p.add_argument("--from-file", dest="from_file", help="read additional sources, one per line (# comments)")
    p.add_argument("--recursive", action="store_true", help="descend into subdirectories for directory sources")
    p.add_argument("--title", help="title override (single-source runs only)")
    p.add_argument("--key-prefix", default="video", help="object key prefix (default: video)")
    p.add_argument("--cookies-from-browser", dest="cookies", default="chrome",
                   help="browser for yt-dlp cookies (default: chrome; URL sources)")
    p.add_argument("--format-sort", default="vcodec:h264,acodec:aac",
                   help="yt-dlp -S string (default prefers H.264/AAC)")
    p.add_argument("--transcode", action="store_true",
                   help="re-encode non-H.264/AAC inputs (default: warn + upload as-is)")
    p.add_argument("--sink", choices=["print", "mytv"], default="print", help="output sink")
    p.add_argument("--channel", type=int, help="MyTV channel id (required with --sink mytv)")
    p.add_argument("--dry-run", action="store_true", help="print planned actions; no download/upload/register")
    args = p.parse_args()

    sources = list(args.sources)
    if args.from_file:
        with open(args.from_file) as f:
            sources += parse_source_list(f.read())
    if not sources:
        die("error: no sources given (pass SOURCE args and/or --from-file)")
    if args.title and len(sources) > 1:
        die("error: --title only applies to a single source")
    if args.sink == "mytv" and args.channel is None:
        die("error: --sink mytv requires --channel")

    require_env("PUBLISH_VIDEO_S3_ENDPOINT", "PUBLISH_VIDEO_S3_BUCKET", "PUBLISH_VIDEO_PUBLIC_BASE_URL")
    endpoint = os.environ["PUBLISH_VIDEO_S3_ENDPOINT"]
    bucket = os.environ["PUBLISH_VIDEO_S3_BUCKET"]
    public_base = os.environ["PUBLISH_VIDEO_PUBLIC_BASE_URL"]

    try:
        jobs = resolve_jobs(sources, args.recursive)
    except ValueError as e:
        die(f"error: {e}")
    if not jobs:
        die("error: no video files found in the given sources")

    if args.dry_run:
        results = [plan_job(s, t, args.key_prefix, public_base, args.title, args.transcode,
                            uuid.uuid4().hex) for s, t in jobs]
        print(json.dumps(build_envelope(results), indent=2))
        return

    for tool in sorted(required_tools(jobs, args.transcode)):
        require_tool(tool)
    if args.sink == "mytv":
        require_env("MYTV_BASE_URL", "MYTV_ADMIN_PASSWORD")

    results = []
    for source, stype in jobs:
        try:
            result = process_job(source, stype, args, endpoint, bucket, public_base)
            if args.sink == "mytv":
                item = register_item(os.environ["MYTV_BASE_URL"], args.channel,
                                     os.environ["MYTV_ADMIN_PASSWORD"],
                                     build_payload(result["title"], result["public_url"],
                                                   result["duration_secs"]))
                result["mytv_item"] = item.get("id", item)
        except PublishError as e:
            result = error_result(source, stype, str(e))
        results.append(result)

    print(json.dumps(build_envelope(results), indent=2))
    sys.exit(exit_code_for(results))
```

Also delete the now-unused `fetch_title` early-return helpers only if they conflict — `fetch_title`, `build_register_url`, `build_payload`, `public_url`, `object_key`, `sanitize_filename`, `build_ytdlp_cmd`, `download_and_mux`, `upload_to_bucket` are all still used. **Update `upload_to_bucket`'s signature** to accept a content type:

```python
def upload_to_bucket(path: str, endpoint: str, bucket: str, key: str, content_type: str = "video/mp4"):
    try:
        import boto3
    except ImportError:
        raise PublishError("boto3 is required for upload (pip install boto3)")
    client = boto3.client("s3", endpoint_url=endpoint)
    client.upload_file(path, bucket, key, ExtraArgs={"ContentType": content_type})
```

- [ ] **Step 4: Run the unit tests — verify pass**

Run: `cd $SCRIPTS && python3 -m unittest test_publish_video -v`
Expected: all tests `ok`.

- [ ] **Step 5: Manual dry-run verification for every input type**

```bash
cd $SCRIPTS
export PUBLISH_VIDEO_S3_ENDPOINT=https://example.r2.cloudflarestorage.com
export PUBLISH_VIDEO_S3_BUCKET=demo
export PUBLISH_VIDEO_PUBLIC_BASE_URL=https://media.example.com
touch /tmp/Local\ Clip.mp4
python3 publish_video.py "https://www.bilibili.com/video/BV1xx" /tmp/Local\ Clip.mp4 "https://cdn.example.com/a.mp4" --dry-run
```
Expected: a JSON envelope with `ok`/`failed` and three `results`, each `"dry_run": true`, types `ytdlp_url`, `local_file`, `direct_url`, and a `public_url` per item. No network calls.

- [ ] **Step 6: Commit**

```bash
cd $PLUGIN && git add -A
git commit -m "feat: orchestration — process_job, batch main(), sinks, neutral env, dry-run"
```

---

### Task 9: Author the skill (`SKILL.md` + `REFERENCE.md`)

**Files:**
- Create: `$PLUGIN/skills/publish-video/SKILL.md`
- Create: `$PLUGIN/skills/publish-video/REFERENCE.md`

- [ ] **Step 1: Write `SKILL.md`**

```markdown
---
name: publish-video
description: Use when you need to publish a local or remote video — a file, a direct media URL, a yt-dlp-supported site URL, or a folder of videos — to a public URL. Downloads/normalizes to a browser-playable H.264/AAC MP4, uploads to S3-compatible object storage, and returns the public URL. Optionally registers it as a MyTV VOD playlist item.
---

# publish-video

Publish one or more videos to a public URL via S3-compatible object storage.

## When to use
- You need a hosted, browser-playable MP4 URL for a video (to embed, share, or feed another system).
- Sources can be: a local file, a local folder, a direct `https://…/x.mp4` link, or any yt-dlp-supported site (YouTube, Bilibili, etc.).

## When NOT to use
- You need HLS/DASH manifest hosting, or private/signed delivery (this tool is public-read only).

## Prerequisites
- `python3` with `boto3` (`pip install boto3`)
- `yt-dlp` (only for site URLs), `ffmpeg`/`ffprobe` (ffprobe always; ffmpeg only with `--transcode`)
- Required env: `PUBLISH_VIDEO_S3_ENDPOINT`, `PUBLISH_VIDEO_S3_BUCKET`, `PUBLISH_VIDEO_PUBLIC_BASE_URL`, plus `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY`. For the MyTV sink only: `MYTV_BASE_URL`, `MYTV_ADMIN_PASSWORD`.

## How to invoke
```bash
python3 ${CLAUDE_PLUGIN_ROOT}/skills/publish-video/scripts/publish_video.py <source> [more sources…] [options]
```

## How to read the result
The script prints a JSON envelope to **stdout** (logs go to stderr):
```json
{ "ok": 1, "failed": 0, "results": [ { "public_url": "https://…/video/<id>-<name>.mp4", "duration_secs": 193, ... } ] }
```
For each entry in `results`, read `public_url` (success) or `error` (failure). Exit code is 0 if all succeeded, 1 if any failed, 2 on a config/usage error.

## Examples
```bash
# yt-dlp site URL (downloads + muxes to H.264/AAC):
python3 …/publish_video.py "https://www.bilibili.com/video/BV1xx"

# Direct media URL (downloaded as-is):
python3 …/publish_video.py "https://cdn.example.com/clip.mp4"

# Local file with a title override:
python3 …/publish_video.py ./movie.mkv --title "Movie" --transcode

# A whole folder, recursively:
python3 …/publish_video.py ~/Videos/exports --recursive

# Batch from a list, register into MyTV channel 7:
python3 …/publish_video.py --from-file urls.txt --sink mytv --channel 7

# Preview without doing anything:
python3 …/publish_video.py ./a.mp4 --dry-run
```

See `REFERENCE.md` for the full flag/env table and JSON schema.
```

- [ ] **Step 2: Write `REFERENCE.md`**

```markdown
# publish-video — Reference

## Flags
| Flag | Default | Purpose |
|------|---------|---------|
| `SOURCE…` (positional) | — | One or more: yt-dlp URL, direct media URL, local file, local directory |
| `--from-file FILE` | — | Read additional sources, one per line (`#` comments, blanks ignored) |
| `--recursive` | off | Descend into subdirectories for directory sources |
| `--title TITLE` | derived | Title override (single-source runs only) |
| `--key-prefix PREFIX` | `video` | Object key prefix |
| `--cookies-from-browser B` | `chrome` | Browser for yt-dlp cookies (URL sources) |
| `--format-sort SORT` | `vcodec:h264,acodec:aac` | yt-dlp `-S` string |
| `--transcode` | off | Re-encode non-H.264/AAC inputs to H.264/AAC (else warn + upload as-is) |
| `--sink {print,mytv}` | `print` | Output sink; `mytv` also registers a playlist item |
| `--channel N` | — | MyTV channel id (required with `--sink mytv`) |
| `--dry-run` | off | Print planned actions; no download/upload/register |

## Environment
| Var | When | Purpose |
|-----|------|---------|
| `PUBLISH_VIDEO_S3_ENDPOINT` | always | S3-compatible endpoint URL |
| `PUBLISH_VIDEO_S3_BUCKET` | always | Bucket name |
| `PUBLISH_VIDEO_PUBLIC_BASE_URL` | always | Public base URL of the bucket |
| `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` | always | boto3 credentials |
| `MYTV_BASE_URL` / `MYTV_ADMIN_PASSWORD` | `--sink mytv` | MyTV API base + admin password |

## JSON output (stdout)
```json
{
  "ok": 2,
  "failed": 1,
  "results": [
    {"source": "...", "type": "ytdlp_url", "title": "...", "public_url": "https://...",
     "object_key": "video/<id>-<name>.mp4", "duration_secs": 193,
     "passthrough": false, "transcoded": false},
    {"source": "...", "type": "local_file", "error": "ffprobe failed: ..."}
  ]
}
```
With `--sink mytv`, successful items also carry `"mytv_item": <id>`. With `--dry-run`, each result carries `"dry_run": true` and a planned `object_key`/`public_url`.

## Exit codes
- `0` — all items succeeded
- `1` — at least one item failed (others still processed)
- `2` — config/usage error (missing env/tool, bad arguments)
```

- [ ] **Step 3: Commit**

```bash
cd $PLUGIN && git add -A
git commit -m "docs: SKILL.md + REFERENCE.md for publish-video"
```

---

### Task 10: Plugin + marketplace manifests + README

**Files:**
- Create: `$PLUGIN/.claude-plugin/plugin.json`
- Create: `$PLUGIN/.claude-plugin/marketplace.json`
- Create: `$PLUGIN/README.md`

- [ ] **Step 1: Write `plugin.json`**

```json
{
  "name": "publish-video",
  "version": "0.1.0",
  "description": "Publish a local or remote video to a public URL: normalize to a browser-playable MP4, upload to S3-compatible object storage, return the URL.",
  "author": { "name": "Kun Wu" }
}
```

- [ ] **Step 2: Write `marketplace.json`**

```json
{
  "name": "kunwu-plugins",
  "owner": { "name": "Kun Wu" },
  "plugins": [
    {
      "name": "publish-video",
      "source": "./",
      "description": "Publish a local or remote video to a public URL via S3-compatible storage."
    }
  ]
}
```

- [ ] **Step 3: Write `README.md`**

```markdown
# publish-video

A Claude Code plugin: a skill that publishes a video (local file, direct URL, yt-dlp-supported site, or folder) to a public URL on S3-compatible object storage, returning the URL as JSON. Optionally registers the result as a MyTV VOD item.

## Install (local)
```bash
/plugin marketplace add /absolute/path/to/publish-video-plugin
/plugin install publish-video
```

## Prerequisites & usage
See `skills/publish-video/SKILL.md` and `skills/publish-video/REFERENCE.md`.

## Tests
```bash
cd skills/publish-video/scripts && python3 -m unittest test_publish_video -v
```
```

- [ ] **Step 4: Verify JSON validity**

Run: `cd $PLUGIN && python3 -c "import json; json.load(open('.claude-plugin/plugin.json')); json.load(open('.claude-plugin/marketplace.json')); print('ok')"`
Expected: `ok`.

- [ ] **Step 5: Commit**

```bash
cd $PLUGIN && git add -A
git commit -m "feat: plugin.json, marketplace.json, README"
```

---

### Task 11: Local install + end-to-end validation

This task is manual (no unit test). It proves an agent can install and call the skill.

- [ ] **Step 1: Add the local marketplace and install**

In a Claude Code session:
```
/plugin marketplace add /Users/<you>/Workspace/playground/publish-video-plugin
/plugin install publish-video
```
Restart the session if the skill does not appear immediately (MCP/skill registration sometimes needs a fresh session).

- [ ] **Step 2: Dry-run via the skill for each input type**

Ask the agent to invoke the skill with `--dry-run` on: a yt-dlp URL, a direct `.mp4` URL, a local file, and a local directory. Confirm the JSON envelope lists the correct `type` per item and a planned `public_url`. (Set the three `PUBLISH_VIDEO_S3_*` env vars to dummy values for dry-run.)

- [ ] **Step 3: One real run against R2**

With a real public-read R2 bucket and `AWS_*` + `PUBLISH_VIDEO_S3_*` set, publish one small local MP4:
```bash
python3 $SCRIPTS/publish_video.py /tmp/bili_demo.mp4 --title "Demo"
```
Expected: JSON with `ok: 1`, a real `public_url`. Open the URL in a browser and confirm it plays.

- [ ] **Step 4: One real run with the MyTV sink (optional)**

With `MYTV_BASE_URL` + `MYTV_ADMIN_PASSWORD` set:
```bash
python3 $SCRIPTS/publish_video.py /tmp/bili_demo.mp4 --title "Demo" --sink mytv --channel 7
```
Expected: the result carries `mytv_item` and the item appears on channel 7.

- [ ] **Step 5: Record validation outcome**

Append a short "Validated <date>" note to `README.md` summarizing what was confirmed, and commit:
```bash
cd $PLUGIN && git add -A && git commit -m "docs: record local end-to-end validation"
```

---

### Task 12: Publish to GitHub, re-point marketplace, drop the seed branch

- [ ] **Step 1: Create the GitHub repo and push**

```bash
cd $PLUGIN
gh repo create publish-video-plugin --public --source=. --remote=origin --push
```

- [ ] **Step 2: Re-point the marketplace to the remote and re-validate**

```
/plugin marketplace remove kunwu-plugins
/plugin marketplace add <github-owner>/publish-video-plugin
/plugin install publish-video
```
Confirm a dry-run still works from the remotely-installed copy.

- [ ] **Step 3: Drop the now-superseded MyTV seed branch**

The tool's work now lives in the plugin repo. Remove the stale branch in MyTV:
```bash
git -C ~/Workspace/playground/MyTV branch -D feat/vod-upload-tool
```
(MyTV retains only the deferred direct-play change — Tasks 1–2 of `2026-06-14-self-hosted-mp4-vod-object-storage.md` — which is unaffected.)

- [ ] **Step 4: Final confirmation**

Confirm: `gh repo view --web` shows the repo; `/plugin` lists `publish-video` as installed from the GitHub marketplace; a dry-run returns a valid JSON envelope.

---

## Notes for the implementer

- **Keep stdout pure JSON.** All progress/warnings go to stderr (the agent contract depends on parsing stdout). The `warning:` print in `ensure_playable` and yt-dlp's own output already go to stderr.
- **`object_key` already prepends the uuid** as `{prefix}/{uid}-{safe}`; `sanitize_filename` turns spaces/specials into `_`. The Task 8 test `video/ID-My_Clip.mp4` reflects that.
- **Local files are never deleted** — `acquire` returns the original path for `local_file`; only the temp `workdir` (downloads/transcodes) is removed.
- **`requests` is not used** — direct download uses stdlib `urllib`; no new dependency beyond boto3.
