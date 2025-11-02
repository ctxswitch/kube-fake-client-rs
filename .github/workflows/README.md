# GitHub Actions Workflows

This directory contains the CI/CD workflows for the project.

## Workflows

### CI (`ci.yml`)

Runs on every push to `main` and on all pull requests.

**Jobs:**

1. **Test** - Runs on multiple platforms (Ubuntu, macOS, Windows) and Rust versions (stable, beta)
   - Checks code compilation
   - Runs unit tests
   - Runs doc tests

2. **Lint** - Code quality checks
   - Checks code formatting with `rustfmt`
   - Runs `clippy` linter with warnings as errors

3. **Coverage** - Code coverage analysis
   - Generates coverage report using `tarpaulin`
   - Uploads to Codecov (optional)

4. **Security** - Security audit
   - Runs `cargo audit` to check for known vulnerabilities

5. **Docs** - Documentation checks
   - Ensures documentation builds without warnings

### Release (`release.yml`)

Runs when a version tag (e.g., `v0.1.0`) is pushed.

**Jobs:**

1. **Release** - Creates a GitHub release
   - Generates changelog from git commits
   - Creates release notes
   - Attaches artifacts

2. **Publish** - Publishes to crates.io
   - Requires `CARGO_TOKEN` secret to be set
   - Automatically triggered after release is created

### Update PR Stack (`update-pr-stack.yml`)

Automatically updates PR descriptions with stack information when using stacked PRs.

**Triggers:**
- When a PR is opened, synchronized, or reopened
- Manual trigger via `workflow_dispatch`

**What it does:**
1. Detects if a PR is part of a stack (based on branch relationships)
2. Builds a visual tree showing the stack hierarchy
3. Updates the PR description with the stack information
4. Adds a comment notifying that stack was detected

**Example stack visualization in PR:**
```markdown
## Stack Information

This PR is part of a stack:

#123 - `feature/base` - Add database schema
  **→ #124** - `feature/api` - Add API endpoints
    #125 - `feature/frontend` - Add UI components
```

**How it works:**
- Analyzes PR base branches to determine parent-child relationships
- PRs that base on `main` are stack roots
- PRs that base on other feature branches are part of a stack
- Automatically highlights the current PR in the tree

**Manual trigger:**
You can manually update all open PRs:
1. Go to Actions → Update PR Stack Info
2. Click "Run workflow"
3. Select the branch (usually `main`)
4. Click "Run workflow"

## Setup Instructions

### Required Secrets

For the release workflow to publish to crates.io, you need to set up the `CARGO_TOKEN` secret:

1. Get your crates.io API token from https://crates.io/settings/tokens
2. Add it to your GitHub repository:
   - Go to Settings → Secrets and variables → Actions
   - Click "New repository secret"
   - Name: `CARGO_TOKEN`
   - Value: Your crates.io token

### Optional: Codecov Integration

To enable code coverage reports:

1. Sign up at https://codecov.io
2. Connect your GitHub repository
3. No additional secrets required (uses `GITHUB_TOKEN`)

### Permissions

The workflows require the following permissions (configured in each workflow file):

- **CI**: `contents: read`
- **Release**: `contents: write`, `pull-requests: write`
- **Update PR Stack**: `pull-requests: write`, `contents: read`

## Release Process

1. Update version in `Cargo.toml`
2. Update `CHANGELOG.md` (if you have one)
3. Commit changes
4. Create and push a tag:
   ```bash
   git tag -a v0.1.0 -m "Release v0.1.0"
   git push origin v0.1.0
   ```
5. GitHub Actions will automatically:
   - Run all CI checks
   - Create a GitHub release
   - Publish to crates.io

Or use the Makefile:
```bash
make version-patch  # Bump version
make tag           # Create git tag
git push origin vX.Y.Z
```

## Stacked PRs Workflow

This project supports stacked PRs (pull requests that depend on each other).

### Creating a Stack

```bash
# Create base PR
git checkout -b feature/base main
# ... make changes ...
git push -u origin feature/base
gh pr create --base main --title "Base changes"

# Create dependent PR
git checkout -b feature/dependent feature/base
# ... make changes ...
git push -u origin feature/dependent
gh pr create --base feature/base --title "Dependent changes"
```

### Automatic Updates

The `update-pr-stack.yml` workflow will automatically:
1. Detect the stack relationship
2. Update both PRs with the stack hierarchy
3. Keep the information synchronized as PRs are updated

### Using Git Town (Optional)

For advanced stack management, you can use [Git Town](https://www.git-town.com/):

```bash
# Install git-town
brew install git-town

# Create stacked branches
git town append feature/base
git town append feature/dependent

# Sync the stack
git town sync
```

The `.git-town.toml` configuration is included in the repository.

## Local Testing

To test workflows locally, you can use [act](https://github.com/nektos/act):

```bash
# Install act
brew install act  # macOS
# or download from https://github.com/nektos/act

# Run CI workflow
act pull_request

# Run specific job
act -j test
```

## Caching

All workflows use GitHub Actions cache to speed up builds:
- Cargo registry
- Cargo index
- Compiled dependencies

This significantly reduces build times on subsequent runs.

## Workflow Best Practices

### For Contributors

1. **PRs**: Create focused PRs with clear descriptions
2. **Stacks**: Use stacked PRs for complex features that need multiple review rounds
3. **CI**: Ensure all CI checks pass before requesting review
4. **Updates**: Keep your PR branch up-to-date with the base branch

### For Maintainers

1. **Review**: Review stack PRs from bottom to top (base first)
2. **Merge**: Merge base PRs before dependent ones
3. **Release**: Follow the release checklist in RELEASE.md
4. **Security**: Respond to security audit failures promptly

## Troubleshooting

### Stack Info Not Updating

If the stack information doesn't update automatically:

1. Check the workflow run in Actions tab
2. Ensure the PR is actually part of a stack (bases on another branch)
3. Manually trigger the workflow:
   - Actions → Update PR Stack Info → Run workflow

### CI Failures

Common issues:

1. **Tests Failed**: Run `cargo test` locally to reproduce
2. **Clippy Warnings**: Run `cargo clippy` and fix warnings
3. **Format Issues**: Run `cargo fmt` to format code
4. **Outdated Dependencies**: Run `cargo update` and test

### Release Workflow Failed

Check the [Release Troubleshooting](../../RELEASE.md#troubleshooting) section in RELEASE.md.
