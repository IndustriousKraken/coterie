/** @type {import('tailwindcss').Config} */
module.exports = {
  content: [
    "./templates/**/*.html",
    // HTMX fragment responses built in Rust with format!()
    "./src/web/**/*.rs",
    "./src/api/**/*.rs",
  ],
  theme: {
    extend: {},
  },
  plugins: [],
}
