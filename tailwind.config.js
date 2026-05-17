/** @type {import('tailwindcss').Config} */
export default {
  darkMode: "class",
  content: ["./index.html", "./src/**/*.{js,ts,jsx,tsx}"],
  theme: {
    extend: {
      colors: {
        background: "#FDFDFF",
        tangerine: {
          DEFAULT: "#F28500",
          50: "#FEF3E2",
          100: "#FCE5C0",
          500: "#F28500",
          600: "#D97600",
          700: "#B86200",
        },
        slate: {
          DEFAULT: "#797D81",
          100: "#F5F5F7",
          400: "#797D81",
          600: "#4B4F53",
          900: "#1A1A1A",
        },
        success: "#10B981",
        danger: "#EF4444",
      },
      fontFamily: {
        sans: [
          "Inter",
          "ui-sans-serif",
          "system-ui",
          "-apple-system",
          "Segoe UI",
          "Roboto",
          "sans-serif",
        ],
      },
      fontWeight: {
        // Slightly bolder, more modern headers
        heading: "650",
      },
      letterSpacing: {
        tightish: "-0.015em",
      },
      boxShadow: {
        soft: "0 10px 30px -12px rgba(24, 24, 27, 0.08), 0 4px 12px -6px rgba(24, 24, 27, 0.05)",
      },
      borderRadius: {
        "2xl": "1rem",
      },
    },
  },
  plugins: [],
};
