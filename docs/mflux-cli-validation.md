# Standalone mflux CLI validation

A local smoke check succeeded on Apple M4 Pro / 48 GiB using an existing isolated mflux environment. This verifies standalone mflux, not a completed gendock generation adapter.

- mflux: 0.18.0; MLX: 0.31.1; Python runtime: 3.11 (private Pinokio environment).
- Model: `flux2-klein-4b-q4`, revision `794cd159538149ad9830848508c31f0ea7088e58`.
- Weight metadata reports mflux 0.19.0 / 4-bit. The tested 0.18.0 runtime successfully loaded this specific snapshot; do not generalize compatibility to every version or variant.
- Download: about 4.30 GiB. Official HF direct connectivity reset on this network; direct HF Mirror worked with proxies unset and `NO_PROXY=*`. No model downloads through a network proxy were needed.
- Files were acquired with temporary parallel HTTP range staging, then verified and activated using gendock's checksums. Parallel staging is an experiment helper, not a shipped CLI feature.
- Text-to-image: 512×512, four steps, seed 42, watercolor fox and blue cup. Entire CLI invocation measured 12.42 seconds, exit 0.
- Image editing: same local model, four steps, change cup/saucer from blue to green; exit 0 and visual result checked.
- Both generation invocations used `HF_HUB_OFFLINE=1`. No extra downloads were needed.
- This runtime's `--metadata` sidecars contained JSON `null`; the generation command/exit/time were captured separately in an experiment run record. Do not assume the sidecar has a complete reproducibility payload.

These are smoke checks, not a broad benchmark or a universal hardware recommendation. Both synchronous `gendock image generate` and `gendock image edit` are now integrated and tested through registered private components. Model defaults come from XDG TOML; automatic runtime installation remains unfinished. See [image-generation.md](image-generation.md).

## Repeat with an installed mflux CLI

```sh
MODEL="${XDG_DATA_HOME:-$HOME/.local/share}/gendock/models/flux2-klein-4b-q4/794cd159538149ad9830848508c31f0ea7088e58"

HF_HUB_OFFLINE=1 mflux-generate-flux2 \
  --model "$MODEL" --base-model flux2-klein-4b \
  --prompt "A watercolor illustration of a red fox beside a blue teacup" \
  --seed 42 --steps 4 --width 512 --height 512 \
  --output fox.png --metadata

HF_HUB_OFFLINE=1 mflux-generate-flux2-edit \
  --model "$MODEL" --base-model flux2-klein-4b \
  --image-paths fox.png \
  --prompt "Change the blue cup to green. Keep everything else unchanged." \
  --seed 42 --steps 4 --width 512 --height 512 \
  --output fox-edited.png --metadata
```

For strict proxy bypass on downloads, unset upper- and lowercase HTTP/HTTPS/ALL proxy variables, and set both NO_PROXY/no_proxy to `*`. Selecting `--endpoint hf-mirror` is independent of proxy configuration.

The existing mflux commands were outside PATH in another application's private environment. Local convenience symlinks exposed them without altering package dependencies. This is a machine-specific experiment; production gendock must still manage its own pinned runtime rather than depend on Pinokio paths.
