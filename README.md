# Coterie

Coterie is a secure, lightweight member management system designed for small to medium-sized groups, clubs, and organizations. Built with security and maintainability in mind, it provides a simple yet powerful platform for managing memberships without the complexity of enterprise solutions.

## Overview

Coterie is a member management system for clubs, groups, social organizations etc. 
You can connect it to your website to verify dues payments and register new members, 
and for members to self-service their accounts. Admins can use Coterie to check 
member status, activate/approve memberships, and update member details.

At its core, Coterie strives to do one thing very well: to make sure you know who is in your group, and who is not.

## Technology Stack

- **Backend**: Rust (using Axum or Actix-web framework)
- **Database**: SQLite with WAL mode
- **Frontend**: HTMX + Alpine.js for minimal, secure interfaces
- **Authentication**: Session-based with secure cookies, Argon2id for password hashing, TOTP for 2FA
- **Deployment**: Single binary deployment with Caddy reverse proxy

## Core Features

### Currently Planned
- **Member Management**: Track active, expired, and pending members
- **Payment Integration**: Stripe integration for dues (no card details stored)
- **Public API**: For member signup and verification from static websites
- **Admin Dashboard**: Manage members, view audit logs, configure settings
- **Calendar System**: Manage events with public/member-only visibility
- **Public Achievements**: Display meeting info, CTF results, member accomplishments
- **RSS Feeds**: For public announcements and member blog aggregation

### Integration System
Coterie uses a modular plugin architecture for third-party integrations:
- **Discord**: Automatically manage member roles based on dues status
- **Unifi Access**: Grant/revoke physical access to facilities
- **VPN/Network**: Manage WireGuard VPN access for lab resources

### Planned Features
- **Expense Tracking**: Track and report group expenses with transparency reports
- **Member Directory**: Opt-in skills/expertise directory
- **Resource Library**: Share tools, guides, and writeups with access controls
- **Voting System**: For group decisions and polls
- **Equipment Checkout**: Track shared hardware and tools
- **Audit Logging**: Complete trail of all administrative actions
- **Webhook System**: For custom integrations (future release)
