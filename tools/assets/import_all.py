"""Import every pick in tools/assets/picks.txt into assets/ via `rusting asset import`.

Usage: python -I tools/assets/import_all.py [--dry-run]
Already-imported files are skipped, so re-running is safe.
"""
import json
import pathlib
import shutil
import subprocess
import sys

TOOLS = pathlib.Path(__file__).resolve().parent
ROOT = TOOLS.parents[1]
DL = ROOT / ".downloads"
STAGE = DL / "stage"
RUSTING = pathlib.Path(r"C:\Users\juliu\RustingEngine\target\release\rusting.exe")


def parse(text):
    """Yield (dest, author, url, srcdir, ext, notes, src, dst) for every pick."""
    section = None
    for line in text.splitlines():
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        if line.startswith("@"):
            fields = [f.strip() for f in line[1:].split("|")]
            section = (fields + [None])[:6]
            continue
        for token in line.split():
            src, _, dst = token.partition("=")
            yield (*section, src, dst or None)


def polypizza_meta():
    meta = {}
    for line in (TOOLS / "polypizza_picks.txt").read_text().splitlines():
        if line and not line.startswith("#"):
            name, public_id, _uuid, creator = line.split()
            meta[name] = (creator.replace("-", " "), f"https://poly.pizza/m/{public_id}")
    return meta


def stage(src, dst, ext):
    """Copy or transcode src to STAGE/dst so the imported file gets the role name."""
    target = STAGE / (dst if pathlib.Path(dst).suffix else dst + ext)
    target.parent.mkdir(parents=True, exist_ok=True)
    if target.suffix == ext:
        shutil.copyfile(src, target)
    else:
        subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", str(src), "-c:a", "libvorbis", "-q:a", "4", str(target)],
                       check=True)
    return target


def main(dry_run):
    pp = polypizza_meta()
    picks = list(parse((TOOLS / "picks.txt").read_text()))
    done, failed = 0, []
    for dest, author, url, srcdir, ext, notes, src, dst in picks:
        if srcdir == "pp":
            author, url = pp[src]
        source = DL / srcdir / (src + ext)
        name = (dst if pathlib.Path(dst).suffix else dst + ext) if dst else source.name
        if (ROOT / "assets" / dest / name).exists():
            continue
        if not source.is_file():
            failed.append(f"{source}: missing")
            continue
        if dst:
            source = stage(source, dst, ext)
        cmd = [str(RUSTING), "asset", "import", str(ROOT), str(source), "--to", dest,
               "--license", "CC0-1.0", "--author", author, "--url", url, "--json"]
        if notes:
            cmd += ["--notes", notes]
        if dry_run:
            cmd.append("--dry-run")
        out = subprocess.run(cmd, capture_output=True, text=True)
        try:
            ok = json.loads(out.stdout).get("ok")
        except json.JSONDecodeError:
            ok = False
        if ok:
            done += 1
        else:
            failed.append(f"{source}: {out.stdout.strip()[:300]} {out.stderr.strip()[:300]}")
    print(f"picks={len(picks)} imported={done} failed={len(failed)}")
    for f in failed:
        print("FAIL", f)
    return 1 if failed else 0


def _selftest():
    rows = list(parse("@ sfx | A B | u | d | .ogg | n\nx=y z # c\n"))
    assert rows == [("sfx", "A B", "u", "d", ".ogg", "n", "x", "y"), ("sfx", "A B", "u", "d", ".ogg", "n", "z", None)], rows


if __name__ == "__main__":
    _selftest()
    sys.exit(main("--dry-run" in sys.argv))
