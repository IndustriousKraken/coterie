/**
 * Site Configuration
 *
 * This file auto-detects the Coterie API URL based on the current hostname.
 * Override the defaults below if you need custom URLs.
 *
 * This file should be loaded BEFORE api.js and main.js.
 */

(function() {
    const hostname = window.location.hostname;
    const protocol = window.location.protocol;

    // =============================================================================
    // AUTO-DETECTION
    // =============================================================================
    //
    // Local development (localhost):
    //   → API: http://localhost:8080
    //
    // Deployed sites:
    //   → API: coterie.{current-hostname}
    //   → Example: stage.foo.bar → coterie.stage.foo.bar
    //   → Example: demo.example.com → coterie.demo.example.com
    //   → Example: myclub.org → coterie.myclub.org

    if (hostname === 'localhost' || hostname === '127.0.0.1') {
        window.COTERIE_API_URL = 'http://localhost:8080';
        window.COTERIE_PORTAL_URL = 'http://localhost:8080';
    } else {
        const coterieUrl = `${protocol}//coterie.${hostname}`;
        window.COTERIE_API_URL = coterieUrl;
        window.COTERIE_PORTAL_URL = coterieUrl;
    }

    // =============================================================================
    // MANUAL OVERRIDES (uncomment and edit to customize)
    // =============================================================================

    // window.COTERIE_API_URL = 'https://api.yourclub.org';
    // window.COTERIE_PORTAL_URL = 'https://members.yourclub.org';

    // Set to null to hide the "Member Portal" link in navigation:
    // window.COTERIE_PORTAL_URL = null;

})();
