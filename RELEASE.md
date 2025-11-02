# Release Process

This document describes the release process for `kube-fake-client`.

## Table of Contents

- [Versioning](#versioning)
- [Release Checklist](#release-checklist)
- [Automated Release Process](#automated-release-process)
- [Manual Release Process](#manual-release-process)
- [Post-Release](#post-release)
- [Troubleshooting](#troubleshooting)

## Versioning

This project follows [Semantic Versioning](https://semver.org/) (SemVer):

- **MAJOR** version (X.0.0): Incompatible API changes
- **MINOR** version (0.X.0): New functionality in a backwards compatible manner
- **PATCH** version (0.0.X): Backwards compatible bug fixes

### Version Bump Guidelines

**Patch Release (0.0.X):**
- Bug fixes
- Documentation updates
- Performance improvements (non-breaking)
- Internal refactoring

**Minor Release (0.X.0):**
- New features
- New public APIs
- Deprecations
- Non-breaking changes to existing functionality

**Major Release (X.0.0):**
- Breaking API changes
- Removal of deprecated features
- Significant architectural changes

## Release Checklist

Before creating a release, ensure:

- [ ] All CI checks pass on `main` branch
- [ ] All tests pass locally: `make test`
- [ ] Code is properly formatted: `make fmt`
- [ ] No clippy warnings: `make clippy`
- [ ] Documentation is up to date
- [ ] CHANGELOG.md is updated (if you maintain one)
- [ ] Version number follows SemVer
- [ ] Examples still work with the new version
- [ ] README.md reflects any API changes

## Automated Release Process

The project uses GitHub Actions for automated releases. This is the **recommended** approach.

### Step 1: Update Version

Use the Makefile to bump the version:

```bash
# For patch release (0.0.X)
make version-patch

# For minor release (0.X.0)
make version-minor

# For major release (X.0.0)
make version-major
```

Or manually edit `Cargo.toml`:

```toml
[package]
version = "0.2.0"  # Update this
```

### Step 2: Update Documentation

Update any version references:

```bash
# Update README examples if needed
# Update doc comments if needed
# Update CHANGELOG.md (recommended)
```

### Step 3: Commit Changes

```bash
git add Cargo.toml Cargo.lock CHANGELOG.md README.md
git commit -m "chore: bump version to 0.2.0"
git push origin main
```

### Step 4: Create and Push Tag

Use the Makefile:

```bash
make tag
# This creates a tag like v0.2.0 and provides the push command
```

Or manually:

```bash
git tag -a v0.2.0 -m "Release v0.2.0"
git push origin v0.2.0
```

### Step 5: Automated Actions

Once the tag is pushed, GitHub Actions will automatically:

1. ✅ Run all CI tests
2. ✅ Build release binaries
3. ✅ Generate changelog from git commits
4. ✅ Create GitHub Release
5. ✅ Publish to crates.io

**Monitor the release:**
- Go to: https://github.com/ctxswitch/kube-fake-client-rs/actions
- Watch the "Release" workflow
- Check: https://github.com/ctxswitch/kube-fake-client-rs/releases
- Verify: https://crates.io/crates/kube-fake-client

## Manual Release Process

If you need to release manually (e.g., automation fails):

### Step 1: Prepare Release

```bash
# Ensure you're on main and up to date
git checkout main
git pull origin main

# Run pre-publish checks
make pre-publish
```

### Step 2: Dry Run

Test the publish process:

```bash
make publish-dry
```

Review the output carefully. This shows what will be published.

### Step 3: Publish to crates.io

```bash
# The Makefile will prompt for confirmation
make publish

# Or directly with cargo
cargo publish
```

### Step 4: Create GitHub Release

1. Go to: https://github.com/ctxswitch/kube-fake-client-rs/releases/new
2. Choose the tag you created
3. Write release notes (see template below)
4. Publish release

### Release Notes Template

```markdown
## What's Changed

- Feature: Add support for X (#123)
- Fix: Resolve issue with Y (#124)
- Docs: Improve documentation for Z (#125)

## Breaking Changes (if any)

- Renamed `old_function()` to `new_function()`
- Removed deprecated `legacy_api()`

## Migration Guide (for breaking changes)

```rust
// Before
client.old_function();

// After
client.new_function();
```

## Installation

```bash
cargo add kube-fake-client@0.2.0
```

Or add to your `Cargo.toml`:
```toml
[dependencies]
kube-fake-client = "0.2.0"
```

**Full Changelog**: https://github.com/ctxswitch/kube-fake-client-rs/compare/v0.1.0...v0.2.0
```

## Post-Release

After a successful release:

### 1. Verify the Release

```bash
# Check crates.io
curl -s https://crates.io/api/v1/crates/kube-fake-client | jq '.crate.max_version'

# Test installation in a new project
cargo new test-release
cd test-release
cargo add kube-fake-client@0.2.0
cargo build
```

### 2. Announce the Release

Consider announcing on:
- GitHub Discussions (if enabled)
- Reddit (r/rust)
- Twitter/X
- Rust Users Forum
- Project Discord/Slack

### 3. Update Documentation

If you maintain external documentation:
- Update docs.rs links
- Update tutorial/guide versions
- Update blog posts or examples

### 4. Monitor Issues

Watch for:
- Bug reports related to the new release
- Questions about new features
- Migration issues (for breaking changes)

## Troubleshooting

### Release Workflow Failed

**Check the workflow logs:**
1. Go to Actions tab
2. Click on the failed workflow
3. Review the error messages

**Common issues:**

1. **Tests Failed**
   ```bash
   # Run tests locally
   cargo test --all-features
   ```

2. **Cargo Token Invalid**
   - Check: Settings → Secrets → CARGO_TOKEN
   - Generate new token: https://crates.io/settings/tokens
   - Update secret in GitHub

3. **Version Mismatch**
   - Ensure Cargo.toml version matches the tag
   - Tag should be `vX.Y.Z` (with 'v' prefix)
   - Cargo.toml should be `X.Y.Z` (without 'v')

### Publish to crates.io Failed

**Manual publish:**

```bash
# Ensure you're logged in
cargo login

# Publish
cargo publish
```

**Common errors:**

1. **"crate version X.Y.Z is already uploaded"**
   - Cannot republish the same version
   - Must bump version and try again

2. **"failed to verify uploaded crate"**
   - Usually a transient error
   - Wait a few minutes and try again

3. **"Authentication required"**
   - Run: `cargo login`
   - Or set: `CARGO_REGISTRY_TOKEN` environment variable

### Rolling Back a Release

**You cannot unpublish from crates.io**, but you can yank:

```bash
# Yank a version (makes it unavailable for new projects)
cargo yank --vers 0.2.0

# Un-yank if needed
cargo yank --vers 0.2.0 --undo
```

**For GitHub releases:**
1. Go to the release page
2. Edit or delete the release
3. The git tag remains (delete separately if needed)

## Release Schedule

This project follows a **as-needed** release schedule:

- **Patch releases**: As needed for critical bugs
- **Minor releases**: When new features are ready
- **Major releases**: When breaking changes accumulate

## Additional Resources

- [crates.io Publishing Guide](https://doc.rust-lang.org/cargo/reference/publishing.html)
- [GitHub Releases](https://docs.github.com/en/repositories/releasing-projects-on-github)
- [Semantic Versioning](https://semver.org/)
- [Keep a Changelog](https://keepachangelog.com/)

## Questions?

If you have questions about the release process:
- Open a Discussion on GitHub
- Ask in the project Discord/Slack
- Contact maintainers directly
