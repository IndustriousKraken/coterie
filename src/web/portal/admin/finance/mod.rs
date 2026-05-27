//! Admin handlers for the expense-tracking surface.
//!
//! Three CRUD endpoints (expenses, categories, accounts) and three
//! reports (monthly / annual reconciliation + tax-prep CSV). All
//! routes are admin-only; the portal router applies the existing
//! `require_admin_redirect` + CSRF middleware tree before routing
//! lands here.

pub mod accounts;
pub mod categories;
pub mod expenses;
pub mod reports;
