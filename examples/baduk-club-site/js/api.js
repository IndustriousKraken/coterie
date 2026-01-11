/**
 * Coterie API Client
 *
 * This module handles all communication with the Coterie backend.
 * Configure COTERIE_API_URL to point to your Coterie instance.
 */

// Configuration - Update this to your Coterie instance URL
const COTERIE_API_URL = window.COTERIE_API_URL || 'http://localhost:8080';

/**
 * Coterie API wrapper
 */
const CoterieAPI = {
    /**
     * Fetch upcoming public events
     * @param {Object} options - Query options
     * @param {number} options.limit - Maximum number of events to return
     * @param {string} options.type - Filter by event type (meeting, workshop, social, etc.)
     * @returns {Promise<Array>} List of events
     */
    async getEvents({ limit = 10, type = null } = {}) {
        const params = new URLSearchParams();
        if (limit) params.set('limit', limit);
        if (type) params.set('type', type);

        const url = `${COTERIE_API_URL}/public/events?${params}`;

        try {
            const response = await fetch(url, {
                headers: {
                    'Accept': 'application/json'
                }
            });

            if (!response.ok) {
                throw new Error(`HTTP ${response.status}: ${response.statusText}`);
            }

            return await response.json();
        } catch (error) {
            console.error('Failed to fetch events:', error);
            throw error;
        }
    },

    /**
     * Fetch public announcements
     * @param {Object} options - Query options
     * @param {number} options.limit - Maximum number of announcements to return
     * @param {string} options.type - Filter by type (news, alert, update, etc.)
     * @returns {Promise<Array>} List of announcements
     */
    async getAnnouncements({ limit = 10, type = null } = {}) {
        const params = new URLSearchParams();
        if (limit) params.set('limit', limit);
        if (type) params.set('type', type);

        const url = `${COTERIE_API_URL}/public/announcements?${params}`;

        try {
            const response = await fetch(url, {
                headers: {
                    'Accept': 'application/json'
                }
            });

            if (!response.ok) {
                throw new Error(`HTTP ${response.status}: ${response.statusText}`);
            }

            return await response.json();
        } catch (error) {
            console.error('Failed to fetch announcements:', error);
            throw error;
        }
    },

    /**
     * Submit a membership signup request
     * @param {Object} data - Signup form data
     * @param {string} data.email - Email address
     * @param {string} data.username - Desired username
     * @param {string} data.full_name - Full name
     * @param {string} data.password - Password
     * @param {string} data.membership_type - Type of membership (standard, student, etc.)
     * @returns {Promise<Object>} Signup result
     */
    async signup(data) {
        const url = `${COTERIE_API_URL}/public/signup`;

        try {
            const response = await fetch(url, {
                method: 'POST',
                headers: {
                    'Content-Type': 'application/json',
                    'Accept': 'application/json'
                },
                body: JSON.stringify(data)
            });

            const result = await response.json();

            if (!response.ok) {
                throw new Error(result.error || `HTTP ${response.status}: ${response.statusText}`);
            }

            return result;
        } catch (error) {
            console.error('Signup failed:', error);
            throw error;
        }
    },

    /**
     * Get the iCal calendar feed URL
     * @returns {string} URL for the calendar feed
     */
    getCalendarFeedUrl() {
        return `${COTERIE_API_URL}/public/feed/calendar`;
    },

    /**
     * Get the RSS feed URL
     * @returns {string} URL for the RSS feed
     */
    getRssFeedUrl() {
        return `${COTERIE_API_URL}/public/feed/rss`;
    },

    /**
     * Health check - verify the API is reachable
     * @returns {Promise<Object>} Health status
     */
    async healthCheck() {
        const url = `${COTERIE_API_URL}/health`;

        try {
            const response = await fetch(url, {
                headers: {
                    'Accept': 'application/json'
                }
            });

            return await response.json();
        } catch (error) {
            console.error('Health check failed:', error);
            throw error;
        }
    }
};

// Export for use in other scripts
window.CoterieAPI = CoterieAPI;
