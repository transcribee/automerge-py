# Python Frontend for Automerge

The Python frontend for [automerge-rs](https://github.com/automerge/automerge-rs)

## Build

```sh
# Install maturin
pip install maturin
# Create venv in "env" folder (required by maturin)
python3 -m venv env
# Activate venv
source ./env/bin/activate

# Build automerge_backend (Python bindings to Rust) and install it as a Python module
maturin develop
```

## Design
These bindings follow a different philosophy from the javascript and old python bindings.
Instead of maintaining two separate document, one on the "rust side" and one on the other language side, the only exists on the "rust side" and bindings are written for the rust API so that python is able to access these bindings in a pythonic way.

## Usage

See `test.py`
