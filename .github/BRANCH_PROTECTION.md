# Branch Protection Rules

This document outlines the required branch protection rules for the Servo.io repository.

## Setting Up Branch Protection

1. Navigate to: **Settings** → **Branches** → **Add branch protection rule**
2. Branch name pattern: `main`

## Required Settings

### Protect matching branches
- [x] **Require a pull request before merging**
  - [x] Require approvals: **1**
  - [x] Dismiss stale pull request approvals when new commits are pushed
  - [x] Require review from Code Owners
  - [ ] Restrict who can dismiss pull request reviews (optional)
  - [x] Allow specified actors to bypass required pull requests (for release automation)
    - Add: `github-actions[bot]`

### Require status checks to pass before merging
- [x] **Require status checks to pass before merging**
  - [x] Require branches to be up to date before merging
  - Required status checks:
    - `Rust Tests`
    - `Security Scan`
    - `Code Coverage`
    - `License & Security Check`
    - `Clippy`

### Additional Protections
- [x] **Require conversation resolution before merging**
- [x] **Require signed commits** (recommended for security)
- [x] **Require linear history** (keeps git history clean)
- [x] **Do not allow bypassing the above settings**
- [x] **Restrict who can push to matching branches**
  - Add: `@servo-team/core-maintainers`

### Force Push and Deletion Protection
- [x] **Prevent force pushes** (everyone, including admins)
- [x] **Prevent deletion** (everyone, including admins)

## Quick Setup via GitHub CLI

You can also configure branch protection via the GitHub CLI:

```bash
# Enable branch protection for main
gh api repos/{owner}/{repo}/branches/main/protection \
  --method PUT \
  --field required_status_checks='{"strict":true,"contexts":["Rust Tests","Security Scan","Code Coverage","License & Security Check","Clippy"]}' \
  --field enforce_admins=true \
  --field required_pull_request_reviews='{"dismissal_restrictions":{},"dismiss_stale_reviews":true,"require_code_owner_reviews":true,"required_approving_review_count":1}' \
  --field restrictions=null \
  --field required_linear_history=true \
  --field allow_force_pushes=false \
  --field allow_deletions=false \
  --field required_conversation_resolution=true
```

## For Development Branch (Optional)

If you use a `develop` branch, apply similar but slightly relaxed rules:

- Branch name pattern: `develop`
- Require approvals: **1**
- Required status checks: Same as main
- Allow force pushes: No
- Allow deletions: No
- Require linear history: Optional

## Rulesets (Modern Alternative)

GitHub now supports Rulesets which offer more flexibility. To use Rulesets:

1. Navigate to: **Settings** → **Rules** → **Rulesets** → **New ruleset**
2. Target branches: `main` (or use pattern `main*`)
3. Enable:
   - Require pull request before merging (1 approval)
   - Require status checks to pass
   - Block force pushes
   - Restrict deletions
   - Require signed commits
   - Require linear history

## Verification

After setup, verify protection is active:

```bash
gh api repos/{owner}/{repo}/branches/main/protection
```

## Notes

- Branch protection rules apply to everyone, including administrators (when "Do not allow bypassing" is enabled)
- For emergency hotfixes, temporarily disable protection or use the bypass mechanism
- Review and update required status checks as your CI/CD evolves
- Consider enabling "Require deployments to succeed" for production environments
