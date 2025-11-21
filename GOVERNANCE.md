# Servo Project Governance

This document outlines the governance model for the Servo project.

## Table of Contents

- [Project Values](#project-values)
- [Roles and Responsibilities](#roles-and-responsibilities)
- [Decision Making](#decision-making)
- [Communication](#communication)
- [Code of Conduct](#code-of-conduct)

## Project Values

The Servo project is guided by the following core values:

1. **Transparency**: All discussions and decisions are made in public
2. **Inclusivity**: Everyone is welcome to contribute
3. **Merit**: Contributions are valued based on their technical merit
4. **Consensus**: We strive for consensus in decision-making
5. **Pragmatism**: We balance idealism with practical considerations

## Roles and Responsibilities

### Contributors

Anyone who contributes to the project in any form (code, documentation, issue reports, etc.) is considered a contributor.

**Responsibilities:**
- Follow the [Code of Conduct](CODE_OF_CONDUCT.md)
- Follow the [Contributing Guidelines](CONTRIBUTING.md)
- Respect the review process

**Rights:**
- Have their contributions reviewed fairly
- Participate in discussions
- Propose changes and new features

### Committers

Committers are contributors who have shown sustained commitment to the project and have been granted write access to the repository.

**Responsibilities:**
- Review pull requests
- Mentor new contributors
- Maintain code quality standards
- Participate in technical discussions
- Help with release management

**Requirements to become a Committer:**
- Consistent high-quality contributions over 3+ months
- Deep understanding of at least one major component
- Demonstrated ability to review others' code
- Alignment with project values
- Nomination by an existing Committer or Maintainer

**How to become a Committer:**
1. An existing Committer or Maintainer nominates the contributor
2. The nomination is discussed among Core Maintainers
3. If there are no objections within 7 days, the nomination passes
4. The new Committer is announced and granted repository access

### Maintainers

Maintainers are committers who take on additional leadership responsibilities for specific components or areas of the project.

**Component Maintainers:**
- `servo-core`: Core workflow and asset orchestration logic
- `servo-runtime`: Execution runtime and retry logic
- `servo-storage`: Storage adapters and metadata management
- `servo-lineage`: Lineage tracking and graph operations
- `servo-cloud-*`: Cloud provider integrations
- `servo-cli`: Command-line interface

**Responsibilities:**
- Everything Committers do, plus:
- Set technical direction for their component
- Make final decisions on technical disagreements
- Ensure their component has adequate test coverage
- Review architectural changes affecting their component
- Participate in security response

**Requirements:**
- Been a Committer for 6+ months
- Deep expertise in the component area
- Demonstrated leadership and mentoring
- Nomination by Core Maintainers

### Core Maintainers

Core Maintainers have overall responsibility for the project's direction, quality, and community health.

**Responsibilities:**
- Everything Maintainers do, plus:
- Set overall project direction
- Make final decisions on project-wide issues
- Manage releases
- Handle security issues
- Ensure project sustainability
- Represent the project publicly

**Current Core Maintainers:**
- Listed in [CODEOWNERS](.github/CODEOWNERS)

**Adding/Removing Core Maintainers:**
- New members require unanimous approval from existing Core Maintainers
- Removal requires 2/3 majority vote
- Members may step down voluntarily at any time

## Decision Making

### Consensus-Based Decision Making

We strive for consensus on all decisions. The process is:

1. **Proposal**: Anyone can propose a change via GitHub Issues or Discussions
2. **Discussion**: Community provides feedback
3. **Refinement**: Proposal is refined based on feedback
4. **Consensus**: If no objections after 7 days, consensus is reached

### When Consensus Cannot Be Reached

If consensus cannot be reached:

1. **Component-level decisions**: Component Maintainer makes the final call
2. **Cross-component decisions**: Core Maintainers vote (simple majority)
3. **Project-wide decisions**: Core Maintainers vote (2/3 majority required)

### Types of Decisions

**Routine Changes** (no special process required):
- Bug fixes
- Documentation improvements
- Test additions
- Code refactoring without behavior changes

**Standard Changes** (require review):
- New features
- API changes (non-breaking)
- Performance improvements
- Dependency updates

**Significant Changes** (require RFC and broader consensus):
- Breaking API changes
- Architectural changes
- New component additions
- Major dependency changes
- Changes to governance

### RFC (Request for Comments) Process

For significant changes:

1. Create an issue with the `RFC` label
2. Provide detailed proposal with:
   - Problem statement
   - Proposed solution
   - Alternatives considered
   - Implementation plan
   - Migration path (if breaking)
3. Allow 2 weeks for community feedback
4. Address concerns and revise proposal
5. Core Maintainers make final decision

## Communication

### Official Channels

- **GitHub Issues**: Bug reports, feature requests
- **GitHub Discussions**: General questions, ideas, announcements
- **GitHub Pull Requests**: Code review and technical discussion
- **Discord**: Real-time chat (link in README)
- **Email**: security@servo.dev for security issues only

### Meetings

**Core Maintainer Meetings:**
- Monthly video call
- Notes published to GitHub Discussions
- Anyone can attend as observer

**Community Calls:**
- Quarterly
- Open to all
- Demo new features, discuss roadmap
- Announced 2 weeks in advance

## Conflict Resolution

1. **Direct Discussion**: Parties attempt to resolve directly
2. **Mediation**: If unresolved, request mediation from a Core Maintainer
3. **Review**: Core Maintainers review and make a decision
4. **Appeal**: Decisions can be appealed to all Core Maintainers

## Code of Conduct

All community members must follow our [Code of Conduct](CODE_OF_CONDUCT.md). Violations will be handled as follows:

1. **Warning**: First offense for minor violations
2. **Temporary Ban**: Repeated minor violations or first major violation
3. **Permanent Ban**: Repeated major violations or egregious behavior

Core Maintainers handle enforcement. Appeals can be sent to conduct@servo.dev.

## Security

Security issues should be reported privately to security@servo.dev. See [SECURITY.md](SECURITY.md) for our full security policy.

## Amendments

This governance document can be amended by:

1. Proposal via GitHub issue with `governance` label
2. Discussion period of at least 2 weeks
3. 2/3 majority vote by Core Maintainers
4. Changes announced broadly to the community

## Acknowledgments

This governance model is inspired by:
- Apache Software Foundation
- Rust Language Project
- Python Enhancement Proposals (PEPs)
- Tokio Project

---

**Version**: 1.0
**Last Updated**: 2025-01-21
**Next Review**: 2025-07-21
