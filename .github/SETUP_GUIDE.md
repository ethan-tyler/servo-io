# Repository Setup Guide

This guide walks you through setting up this repository to world-class open source standards.

## Quick Setup Checklist

- [ ] [Configure Branch Protection](#1-configure-branch-protection)
- [ ] [Set up GitHub Secrets](#2-set-up-github-secrets)
- [ ] [Enable GitHub Features](#3-enable-github-features)
- [ ] [Set up Labels](#4-set-up-labels)
- [ ] [Configure Pre-commit Hooks](#5-configure-pre-commit-hooks)
- [ ] [Test CI/CD Workflows](#6-test-cicd-workflows)
- [ ] [Optional: Enable Additional Services](#7-optional-additional-services)

## 1. Configure Branch Protection

See [BRANCH_PROTECTION.md](BRANCH_PROTECTION.md) for detailed instructions.

### Quick Method (via GitHub CLI):

```bash
gh api repos/ethan-tyler/servo-io/branches/main/protection \
  --method PUT \
  --field required_status_checks='{"strict":true,"contexts":["Rust Tests","Security Scan","Code Coverage","License & Security Check"]}' \
  --field enforce_admins=true \
  --field required_pull_request_reviews='{"dismiss_stale_reviews":true,"require_code_owner_reviews":true,"required_approving_review_count":1}' \
  --field restrictions=null \
  --field required_linear_history=true \
  --field allow_force_pushes=false \
  --field allow_deletions=false \
  --field required_conversation_resolution=true
```

### Via GitHub UI:

1. Go to **Settings** → **Branches** → **Add branch protection rule**
2. Branch name pattern: `main`
3. Enable the protections listed in [BRANCH_PROTECTION.md](BRANCH_PROTECTION.md)

## 2. Set up GitHub Secrets

Add the following secrets in **Settings** → **Secrets and variables** → **Actions**:

### Required Secrets:

```bash
# For code coverage
gh secret set CODECOV_TOKEN

# For publishing to crates.io (when ready)
gh secret set CARGO_TOKEN

# For container registry (already available as GITHUB_TOKEN)
# No action needed
```

### Getting the Tokens:

**CODECOV_TOKEN:**
1. Go to [codecov.io](https://codecov.io)
2. Sign in with GitHub
3. Add your repository
4. Copy the upload token

**CARGO_TOKEN:**
1. Log in to [crates.io](https://crates.io)
2. Go to Account Settings → API Tokens
3. Create a new token with publish permissions

## 3. Enable GitHub Features

### Enable GitHub Pages (for documentation):

```bash
gh api repos/ethan-tyler/servo-io --method PATCH \
  --field has_pages=true
```

Or via UI:
1. **Settings** → **Pages**
2. Source: Deploy from a branch
3. Branch: `gh-pages` / `root`

### Enable Discussions:

```bash
gh api repos/ethan-tyler/servo-io --method PATCH \
  --field has_discussions=true
```

Or via UI:
1. **Settings** → **Features**
2. Check "Discussions"

### Enable Vulnerability Alerts:

1. **Settings** → **Code security and analysis**
2. Enable:
   - Dependency graph
   - Dependabot alerts
   - Dependabot security updates
   - Secret scanning
   - Code scanning (optional, requires GitHub Advanced Security for private repos)

## 4. Set up Labels

Sync the label configuration:

```bash
# Install gh-label if not already installed
gh extension install heaths/gh-label

# Sync labels
gh label sync -f .github/labels.yml
```

Or manually import via GitHub UI:
1. **Issues** → **Labels**
2. Create labels as defined in [labels.yml](.github/labels.yml)

## 5. Configure Pre-commit Hooks

For contributors to use pre-commit hooks locally:

```bash
# Install pre-commit (one-time setup)
pip install pre-commit

# Install the git hooks
pre-commit install

# Install commit-msg hook for conventional commits
pre-commit install --hook-type commit-msg

# Test that it works
pre-commit run --all-files
```

## 6. Test CI/CD Workflows

### Test CI Workflow:

```bash
# Create a test branch
git checkout -b test/ci-setup

# Make a small change
echo "# Test" >> README.md

# Commit and push
git add README.md
git commit -m "test: verify CI workflows"
git push -u origin test/ci-setup

# Create PR
gh pr create --title "Test: CI Setup" --body "Testing CI workflows"
```

Check that all workflows run successfully:
- CI (tests, clippy, fmt)
- Code Coverage
- License & Security Check
- PR Labeler

### Test Release Workflow:

```bash
# Create a test tag (optional, only when ready to release)
git tag -a v0.1.0 -m "Release v0.1.0"
git push origin v0.1.0
```

## 7. Optional: Additional Services

### Codecov Configuration:

Create `codecov.yml` in the root (already done if using our setup).

### Sponsor Button:

Edit [FUNDING.yml](.github/FUNDING.yml) and uncomment relevant lines:

```yaml
github: [your-username]
open_collective: servo
patreon: servo
```

### Readme Badges:

The README badges will work once:
- First CI run completes
- Codecov is set up
- First release is created

Update the repository URLs in README.md if they're incorrect.

## 8. Verify Setup

Run through this checklist:

```bash
# Check branch protection
gh api repos/ethan-tyler/servo-io/branches/main/protection

# Check if workflows are enabled
gh workflow list

# Check secrets are set
gh secret list

# Run a test build locally
cargo build --all-features
cargo test --all-features
cargo clippy --all-targets --all-features

# Test pre-commit
pre-commit run --all-files
```

## 9. Ongoing Maintenance

### Weekly:
- Review Dependabot PRs
- Check security alerts
- Review open issues

### Monthly:
- Review stale issues and PRs
- Update documentation
- Review and update roadmap

### Before Each Release:
- Update CHANGELOG.md
- Update version numbers in Cargo.toml files
- Test release process
- Create GitHub release with notes

## Common Issues

### Issue: CI workflows not running

**Solution:** Check that workflows are enabled in **Actions** tab.

### Issue: Branch protection too strict

**Solution:** Temporarily disable protection to push fixes, then re-enable.

### Issue: Codecov upload fails

**Solution:** Verify `CODECOV_TOKEN` is set correctly in repository secrets.

### Issue: Pre-commit hooks fail locally

**Solution:**
```bash
# Update hooks
pre-commit autoupdate

# Clear cache and retry
pre-commit clean
pre-commit run --all-files
```

## Getting Help

- Check [CONTRIBUTING.md](../CONTRIBUTING.md) for contribution guidelines
- Check [GOVERNANCE.md](../GOVERNANCE.md) for project governance
- Open a GitHub Discussion for questions
- File an issue for bugs or feature requests

## References

- [GitHub Branch Protection](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-protected-branches/about-protected-branches)
- [GitHub Actions](https://docs.github.com/en/actions)
- [Codecov Documentation](https://docs.codecov.io/)
- [Pre-commit Documentation](https://pre-commit.com/)
- [Conventional Commits](https://www.conventionalcommits.org/)
