import argparse
import hashlib
import json
import os
import platform
import shutil
import subprocess
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
DOCKERFILE = ROOT / "extra/linux/Dockerfile.portable"
IMAGE = "termust-portable-builder:rust-1.89-bullseye"


def run(*args, **kwargs):
    return subprocess.run(args, check=True, text=True, **kwargs)


def main():
    parser = argparse.ArgumentParser(description="Build the portable Linux x86_64 binary in a container.")
    parser.add_argument("--cache-dir", type=Path, default=Path(os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache")) / "termust/portable")
    parser.add_argument("--output-dir", type=Path, default=ROOT / "dist/portable")
    args = parser.parse_args()
    if platform.system() != "Linux" or platform.machine() not in ("x86_64", "amd64"):
        raise SystemExit("Portable builds currently require a Linux x86_64 host.")
    if not shutil.which("devrig"):
        raise SystemExit("Install devrig to build the isolated release image.")
    cache = args.cache_dir.expanduser().absolute()
    output = args.output_dir.expanduser().absolute()
    for path in (cache, output):
        path.mkdir(parents=True, exist_ok=True)
        if "," in str(path) or "\n" in str(path):
            raise SystemExit("Build paths must not contain commas or newlines.")
    if "," in str(ROOT) or "\n" in str(ROOT):
        raise SystemExit("The checkout path must not contain commas or newlines.")
    with tempfile.TemporaryDirectory(prefix="termust-builder-image-") as temporary:
        context = Path(temporary)
        shutil.copyfile(DOCKERFILE, context / "Dockerfile")
        build = run("devrig", "build", str(context), "--platform", "linux/amd64", "-t", IMAGE, stdout=subprocess.PIPE)
        result = json.loads(build.stdout)
    engine = result.get("tool")
    if engine not in ("docker", "podman"):
        raise SystemExit("devrig did not report the container engine used to build the image.")
    image_id = run(engine, "image", "inspect", "--format", "{{.Id}}", IMAGE, stdout=subprocess.PIPE).stdout.strip()
    layers = run(engine, "image", "inspect", "--format", "{{json .RootFS.Layers}}", image_id, stdout=subprocess.PIPE).stdout.strip()
    layers_hash = hashlib.sha256(layers.encode()).hexdigest()
    recipe_hash = hashlib.sha256(DOCKERFILE.read_bytes()).hexdigest()
    base_image = DOCKERFILE.read_text().splitlines()[0].removeprefix("FROM ")
    program = '''import hashlib, json, os, pathlib, re, shutil, subprocess
def read(*args):
    return subprocess.check_output(args, text=True).strip()
subprocess.run(["cargo", "build", "--locked", "--release", "-p", "termust"], check=True)
target = pathlib.Path("/output/termust.tmp")
shutil.copyfile("/cache/target/release/termust", target)
target.chmod(0o755)
subprocess.run(["strip", "--strip-unneeded", str(target)], check=True)
requirements = read("readelf", "--version-info", str(target))
versions = set(re.findall(r"GLIBC_(\\d+(?:\\.\\d+)+)", requirements))
minimum = max(versions, key=lambda version: tuple(map(int, version.split("."))))
if tuple(map(int, minimum.split("."))) > (2, 31):
    raise SystemExit("Portable build unexpectedly needs glibc " + minimum)
release = {}
for line in pathlib.Path("/etc/os-release").read_text().splitlines():
    if "=" in line:
        key, value = line.split("=", 1)
        release[key] = value.strip('"')
metadata = {"build_os": release["PRETTY_NAME"], "toolchain": read("rustc", "--version"),
            "minimum_glibc": minimum, "binary_sha256": hashlib.sha256(target.read_bytes()).hexdigest(),
            "binary_version": read(str(target), "--version"), "base_image": os.environ["TERMUST_BASE_IMAGE"],
            "builder_layers_sha256": os.environ["TERMUST_BUILDER_LAYERS_SHA256"],
            "builder_recipe_sha256": os.environ["TERMUST_BUILDER_RECIPE_SHA256"], "target_cpu": "x86-64"}
metadata_path = pathlib.Path("/output/build.json.tmp")
metadata_path.write_text(json.dumps(metadata, indent=2) + "\\n")
target.replace("/output/termust")
metadata_path.replace("/output/build.json")
print(json.dumps(metadata, indent=2))
'''
    run(engine, "run", "--rm", "--platform", "linux/amd64", "--user", f"{os.getuid()}:{os.getgid()}",
        "--mount", f"type=bind,src={ROOT},dst=/source,readonly",
        "--mount", f"type=bind,src={cache},dst=/cache",
        "--mount", f"type=bind,src={output},dst=/output",
        "--env", "CARGO_HOME=/cache/cargo", "--env", "CARGO_TARGET_DIR=/cache/target",
        "--env", "RUSTFLAGS=-C target-cpu=x86-64", "--env", "CARGO_INCREMENTAL=0",
        "--env", "GIT_CONFIG_COUNT=1", "--env", "GIT_CONFIG_KEY_0=safe.directory", "--env", "GIT_CONFIG_VALUE_0=/source",
        "--env", f"TERMUST_BASE_IMAGE={base_image}", "--env", f"TERMUST_BUILDER_LAYERS_SHA256={layers_hash}",
        "--env", f"TERMUST_BUILDER_RECIPE_SHA256={recipe_hash}",
        image_id, "python3", "-c", program)
    metadata = json.loads((output / "build.json").read_text())
    if hashlib.sha256((output / "termust").read_bytes()).hexdigest() != metadata["binary_sha256"]:
        raise SystemExit("Portable output checksum does not match its build record.")
    print(f"Portable binary: {output / 'termust'}")


if __name__ == "__main__":
    main()
