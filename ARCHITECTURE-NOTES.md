# Coterie Architecture Notes & Improvement Suggestions

Based on an analysis of the documentation (`README.md` and `ARCHITECTURE.md`) and the codebase structure, here is a summary of Coterie's purpose, its current architecture, and some suggested architectural improvements.

## 🎯 Project Purpose
**Coterie** is a secure, lightweight member management system tailored for small to medium-sized clubs, groups, and organizations. Its primary goal is to provide a single source of truth for membership status (active, expired, pending).
It handles dues and payments (via Stripe), event RSVP tracking, announcements, and administrative functions. By design, it explicitly avoids being a full CMS, opting instead to power an organization's *existing* static marketing website via public APIs while providing its own internal portal for management.

## 🏛️ Current Architecture
Coterie is built as a **Modular Monolith** using a single Rust binary and a SQLite database. 

**Key Architectural Decisions:**
1. **Dual-Frontend Design**: 
   - **Public Site**: An external static site (e.g., Hugo, Next.js) that consumes Coterie's unauthenticated JSON APIs (`/public/*`).
   - **Management Portal**: An internal, server-side rendered application built into the Rust backend using Axum, HTMX, and Alpine.js (`/portal/*`).
2. **Data Layer**: SQLite running in WAL (Write-Ahead Logging) mode, managed via automated snapshots and backups instead of a traditional client-server DB like PostgreSQL.
3. **Synchronous Side-Effects**: Currently, administrative actions (like approving a member) trigger their side-effects (e.g., sending emails, syncing Discord roles) sequentially within the same HTTP handler.
4. **Security by Default**: Top-level CSRF protection is strictly enforced on all state-changing endpoints, with a narrow, explicit exempt list for webhooks and public cross-origin endpoints.

---

## 💡 Suggestions for Architectural Improvements

The current monolithic, SQLite-backed architecture is an excellent, cost-effective choice for this domain. However, as the system grows, here are a few structural improvements to consider regarding reliability and scaling:

### 1. Adopt an "Outbox Pattern" for Side-Effects
**Current State:** The docs state that the portal handler *"does the full side-effect chain in one place"* (e.g., updating the DB, sending a welcome email, and syncing Discord roles).
**The Risk:** Doing slow network calls to external APIs (Discord, SMTP) during an HTTP request makes the app brittle. If Discord is down, the admin action might fail or partially succeed (DB updates but the role isn't synced), leaving the system in an inconsistent state. Furthermore, if you hold a SQLite transaction open during a network call, you risk locking the database for other writers.
**Improvement:** Implement an **Outbox Pattern**. Write the intent to trigger a side-effect into an `outbox_events` SQLite table within the same transaction that updates the member. A background worker can then poll this table, perform the network calls, and handle retries on failure. This keeps HTTP handlers lightning fast and ensures eventual consistency.

### 2. Introduce a Persistent Background Job Queue
**Current State:** Jobs like `BillingRunner` run in continuous `tokio::spawn` loops that sleep for intervals.
**The Risk:** If the server crashes or restarts, interval tracking is lost. While some tasks track state (like `dues_reminder_sent_at`), in-memory loops are generally harder to monitor, scale, or manually trigger.
**Improvement:** Formalize background tasks using a lightweight, SQLite-backed job queue (similar to how you would use an Outbox). This provides durability, structured retry logic (e.g., exponential backoff for failed Stripe API calls), and clear visibility into job failures.

### 3. Bot Protection for Public APIs
**Current State:** `POST /public/signup` and `POST /public/donate` are explicitly exempt from CSRF so that a cross-origin static site can POST to them.
**The Risk:** Without CSRF, these endpoints are vulnerable to automated bot abuse—specifically fake account creation and credit card testing (carding attacks) on the donation endpoint.
**Improvement:** Integrate a lightweight anti-bot mechanism like Cloudflare Turnstile or Google reCAPTCHA v3. The static site can generate a token, include it in the POST request, and the Rust backend can verify it before processing the signup or Stripe session creation.

### 4. Strict SQLite Connection Management
**The Risk:** SQLite in WAL mode allows multiple readers but only **one concurrent writer**. As background jobs (billing runs) and concurrent admin actions increase, write contention can lead to `SQLITE_BUSY` errors.
**Improvement:** Ensure your database connection pool (likely `sqlx`) is configured with a robust `busy_timeout` (e.g., 5-10 seconds). Moreover, enforce a strict architectural rule: **Never await an external network call while holding a database transaction or a database write lock.**