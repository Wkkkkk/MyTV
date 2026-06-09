#!/usr/bin/env python3
import argparse
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

def clean_vtt(content: str) -> str:
    """
    Clean WebVTT content to plain text.
    Removes headers, timestamps, and duplicate lines.
    """
    lines = content.splitlines()
    text_lines = []
    seen = set()
    
    timestamp_pattern = re.compile(r'\d{2}:\d{2}:\d{2}[.,]\d{3}\s-->\s\d{2}:\d{2}:\d{2}[.,]\d{3}')
    
    for line in lines:
        line = line.strip()
        if not line or line == 'WEBVTT' or line.isdigit():
            continue
        if timestamp_pattern.match(line):
            continue
        if line.startswith('NOTE') or line.startswith('STYLE'):
            continue
            
        if text_lines and text_lines[-1] == line:
            continue
            
        line = re.sub(r'<[^>]+>', '', line)
        
        text_lines.append(line)
        
    return '\n'.join(text_lines)

def get_transcript(url: str, lang: str = "en", cookies_browser: str = None):
    with tempfile.TemporaryDirectory() as temp_dir:
        cmd = [
            "yt-dlp",
            "--write-subs",
            "--write-auto-subs",
            "--skip-download",
            "--sub-lang", lang,
            "--output", "subs",
        ]
        if cookies_browser:
            cmd += ["--cookies-from-browser", cookies_browser]
        cmd.append(url)

        try:
            subprocess.run(cmd, cwd=temp_dir, check=True, capture_output=True)
        except subprocess.CalledProcessError as e:
            print(f"Error running yt-dlp: {e.stderr.decode()}", file=sys.stderr)
            sys.exit(1)
        except FileNotFoundError:
            print("Error: yt-dlp not found. Please install it.", file=sys.stderr)
            sys.exit(1)

        temp_path = Path(temp_dir)
        sub_files = list(temp_path.glob("*.vtt")) + list(temp_path.glob("*.srt"))

        if not sub_files:
            print("No subtitles found.", file=sys.stderr)
            sys.exit(1)

        sub_file = sub_files[0]

        content = sub_file.read_text(encoding='utf-8')
        clean_text = clean_vtt(content)
        print(clean_text)

def main():
    parser = argparse.ArgumentParser(description="Fetch YouTube transcript.")
    parser.add_argument("url", help="YouTube video URL")
    parser.add_argument("--lang", default="en", help="Subtitle language code (e.g. en, ai-zh, ai-en)")
    parser.add_argument("--cookies-from-browser", dest="cookies_browser", default=None,
                        help="Browser to extract cookies from (e.g. chrome, safari)")
    args = parser.parse_args()

    get_transcript(args.url, lang=args.lang, cookies_browser=args.cookies_browser)

if __name__ == "__main__":
    main()
