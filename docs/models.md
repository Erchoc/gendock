# Model catalog and downloads

## Search and inspect

```sh
gendock model search flux
gendock model info --model flux2-klein-4b-q4
```

Search queries the **local SQLite `model_catalog` table**, across IDs, names, families, descriptions and repository names. It searches all model types together; there is no type filter. The old `search --query flux` form remains compatible. Searching is offline and does not download model files, but may initialize the database and reconcile built-in definitions.

Results include `type` (`text`, `image`, `video`, `audio`, `music`, `other`), `origin` (`builtin` or `user`), `family`, `homepage_url`, `download_urls`, status/recommendation, revision, estimated size and hardware compatibility. Info additionally includes the per-file manifest. A homepage introduces the model; a file URL must return actual file bytes, not a web preview page.

The initial release includes two mflux-format FLUX definitions. Private runtime installation is not automated yet. Synchronous generation and editing work for installed FLUX.2 Klein 4B q4 with registered execution components; see [image-generation.md](image-generation.md). Quick readiness checks inspect the recipe/component, hardware and file sizes; full hashes are verified at generation. `recommended` is catalog metadata, not proof that a model is ready on the machine. Compatibility distinguishes `candidate`, `unsupported`, and `unknown`; unknown/custom backends are not falsely marked supported. Local-only references may have no homepage or download manifest; their download URLs are empty and `downloadable` is false.

Readiness is also exposed per operation through `ready_operations`. Defaults are user preferences in XDG `config.toml`, separate from catalog recommendations; see [image-generation.md](image-generation.md).

## Database and release upgrades

Two concerns are deliberately separate:

| Table | Responsibility |
| --- | --- |
| `model_catalog` | Model definitions, origin/type/family, lifecycle and JSON file manifests |
| `models` | Local registrations: path, backend, ownership (legacy table name retained) |
| `catalog_state` | Last reconciled built-in catalog revision |

Before 1.0, all five tables are defined by one development schema snapshot (`migrations/001_initial.sql`), with no automatic structural upgrade chain. Existing compatible development databases are preserved; incompatible ones return an explicit error rather than being reset. The current marker stays 3 for compatibility. Released user databases will get versioned migrations after 1.0. Built-in catalog metadata updates below are independent of schema migration.

The binary bundles `catalog/models.json` as a release seed, not as the search engine. On database access, a newer `BUILTIN_CATALOG_REVISION` triggers a transaction that:

- Adds new built-in definitions and updates existing built-in metadata.
- Marks built-ins omitted by the new release `deprecated` and not recommended, with a reason.
- Preserves user definitions, including a user ID that collides with a newly introduced built-in.
- Never deletes local files, installations or jobs.
- Records the new catalog revision; an older catalog revision cannot revert these decisions.

Maintainers must increment `BUILTIN_CATALOG_REVISION` when changing shipped definitions. Catalog revisions and SQLite schema versions are independent. Users cannot directly edit/delete built-in definitions through manifest commands; clone under a new ID for customization.

Lifecycle: `active` can be recommended or not; `deprecated` remains visible and downloadable but is not recommended; `retired` remains visible but new downloads are rejected. Optional `replaced_by` and `deprecation_reason` tell agents what to use next. Existing files are retained in every case. Replacements are explicit metadata, never guessed or automatically downloaded.

## User-maintained models

Import a JSON definition, then inspect/search/download it like a built-in:

```sh
gendock model add --manifest ./my-model.json
gendock model search my-model
gendock model update --manifest ./my-model.json
gendock model remove --model my-model
```

Example manifest (illustrative URLs and a three-byte fixture, not real model weights):

```json
{
  "id": "my-model",
  "name": "My Image Model",
  "type": "image",
  "family": "my-image-family",
  "description": "A user-maintained image model.",
  "homepage_url": "https://huggingface.co/example/my-model",
  "revision": "v1",
  "license": "unspecified",
  "backend": "custom",
  "status": "active",
  "recommended": false,
  "tasks": ["image.generate"],
  "files": [
    {
      "path": "model.bin",
      "url": "https://models.example.com/my-model/v1/model.bin",
      "size": 3,
      "algorithm": "sha256",
      "hash": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    }
  ]
}
```

A direct URL may point to public R2 storage, another CDN, or the original publisher. Multi-file models need one entry per file; a single URL to a webpage or an unpacked archive does not replace a file manifest. No R2 SDK/credentials are needed to fetch public file URLs. Uploading files to R2/HF is outside this command's scope; importing a manifest registers metadata only (`files_uploaded: false`).

Use a stable model ID for a variant; change `revision` when changing its weights. Downloads are verified against your declared size/hash, not trusted simply because a URL responds. JSON imports are capped at 1 MiB. IDs/revisions must be safe directory names; file paths cannot escape the model directory, overlap file/directory names, collide with temporary `.part` paths, or replace the internal manifest. `origin` from an imported JSON is always normalized to `user`.

Model format/backend validation is still separate: registering an arbitrary file does not make a backend capable of running it.

## Existing local weights

```sh
gendock model add --name flux2-klein-4b-q4 --path /Volumes/AI/my-model
gendock model update --name flux2-klein-4b-q4 --path /Volumes/OtherDisk/my-model
gendock model list
gendock model remove --name flux2-klein-4b-q4
```

`--name` removal detaches only the local registration; the catalog entry remains searchable. `--model` removal deletes a **user** catalog entry and its local registration. Both preserve all files. If a local registration uses an existing catalog ID, it attaches to that definition without overriding its metadata. Models referenced by other applications are never modified.

## Download sources and networking

```sh
gendock model download --model flux2-klein-4b-q4
gendock model download --model flux2-klein-4b-q4 --endpoint hf-mirror
gendock model download --model flux2-klein-4b-q4 --proxy http://127.0.0.1:7890
gendock model download --model flux2-klein-4b-q4 --models-dir /Volumes/AI/gendock/models
```

`files[].url` is used directly. For HF URLs, `--endpoint hf` (default) keeps the official origin; `hf-mirror` changes it to `https://hf-mirror.com`. A custom HTTPS origin is also accepted. Direct R2/CDN URLs are never rewritten by HF endpoint selection. A file without `url` can use the legacy HF resolver when the definition provides a repository and immutable 40-character commit.

`--proxy` configures HTTP/HTTPS/SOCKS5/SOCKS5h networking; it never selects a mirror. Standard proxy environment variables may also be honored. No automatic mirror fallback. HF Mirror is third-party and may not contain the exact revision. The official HF token is sent only to official HF requests, never to R2, mirrors or other custom origins. Redirect handling does not forward authorization across different origins. Use stable public URLs; signed URLs expire and sensitive query values in a user manifest should not be shared.

HTTPS is required except HTTP loopback for tests. Endpoint origins cannot contain credentials, queries or path prefixes. Direct file URLs can contain paths/queries but not userinfo or fragments.

## Download lifecycle

The downloader locks the model directory, checks remaining space on the destination disk, preserves resumable bytes in `<models-root>/<id>/.<revision>.partial/`, validates ranges/sizes/hashes, writes a snapshot manifest and atomically activates the completed revision on the same filesystem. Only then is managed ownership registered in SQLite. Rerunning recovers an activation that completed before registration. A verified existing revision is reused without downloading again.

If a server ignores Range and returns 200, that file restarts safely. Invalid ranges fail without appending. Full corrupt partial files are discarded; incomplete transfers remain resumable. External registrations with the same ID must be detached or renamed before a managed download.

Large HF LFS files use SHA-256; small Git files use Git blob SHA-1 (`blob <size>\0` prefix plus bytes). R2/CDN files should normally use SHA-256. Expected checksums live in the local catalog, not in untrusted response headers. Maintainers populate built-in manifests using official HF metadata at fixed commits and retain model README/license/config/tokenizer files.

Downloads are synchronous and produce one final JSON response. Connections have a timeout and each HTTP request has a five-minute timeout; rerun after a transport failure to resume. Background download progress/cancellation and upload commands are not implemented. Downloaded is not the same as inference-ready.

## Tests

`cargo test --locked` uses local HTTP fixtures and temporary databases: initialization/reopen preservation, incompatible-schema rejection, release reconciliation, user lifecycle, source/token isolation, resume, corruption, concurrency and ownership. No real weights are downloaded. An optional network check fetches only a pinned HF README:

```sh
cargo test --locked --test hf_live -- --ignored
```
