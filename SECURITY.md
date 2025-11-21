# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 0.1.x   | :white_check_mark: |

## Reporting a Vulnerability

**Please do not report security vulnerabilities through public GitHub issues.**

Instead, please report security vulnerabilities via email to: security@servo.dev

Please include:

- Type of issue (e.g., buffer overflow, SQL injection, cross-site scripting)
- Full paths of source file(s) related to the issue
- Location of the affected source code (tag/branch/commit or direct URL)
- Step-by-step instructions to reproduce the issue
- Proof-of-concept or exploit code (if possible)
- Impact of the issue

We will respond within 48 hours and aim to fix critical vulnerabilities within 7 days.

## Security Best Practices

When using Servo:

1. **Never commit credentials** to version control
2. **Use Workload Identity/IAM roles** instead of static credentials
3. **Enable encryption** at rest and in transit
4. **Keep dependencies updated** (use Dependabot)
5. **Follow least privilege principle** for service accounts
6. **Enable audit logging** in production
7. **Use VPC/private subnets** for databases

## Security Updates

Security updates will be released as patch versions and announced via:

- GitHub Security Advisories
- Email to security mailing list (subscribe at security@servo.dev)
- Discord #announcements channel
