/**
 * Site Configuration
 *
 * Edit these values to match your Coterie instance.
 * This file should be loaded BEFORE api.js and main.js.
 */

// =============================================================================
// REQUIRED CONFIGURATION
// =============================================================================

/**
 * URL of your Coterie API server
 * - For local development: 'http://localhost:8080'
 * - For production: 'https://api.yourclub.org' or 'https://yourclub.org'
 *
 * This is used for:
 * - Fetching events and announcements
 * - Member signup form submission
 * - Calendar and RSS feed URLs
 */
window.COTERIE_API_URL = 'http://localhost:8080';

/**
 * URL of the member portal (login page)
 * - Often the same as COTERIE_API_URL
 * - Set to null to hide the "Member Portal" link in navigation
 *
 * Examples:
 * - 'http://localhost:8080' (local dev)
 * - 'https://members.yourclub.org'
 * - 'https://yourclub.org/portal'
 */
window.COTERIE_PORTAL_URL = 'http://localhost:8080';


// =============================================================================
// OPTIONAL CONFIGURATION
// =============================================================================

/**
 * Club name (used in page titles, etc.)
 * Leave null to use the default from HTML
 */
window.SITE_NAME = null;
