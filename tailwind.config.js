/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        bg: "#141414",
        surface: "#1F1F1F",
        "surface-hover": "#2A2A2A",
        primary: "#E50914",
        "primary-hover": "#C40812",
        text: "#FFFFFF",
        muted: "#B3B3B3",
      },
      borderRadius: {
        xl: "0.75rem",
      },
      keyframes: {
        fadein: {
          "0%": { opacity: "0", transform: "translateY(8px)" },
          "100%": { opacity: "1", transform: "translateY(0)" },
        },
        spin: {
          "0%": { transform: "rotate(0deg)" },
          "100%": { transform: "rotate(360deg)" },
        },
      },
      animation: {
        fadein: "fadein 0.3s ease-out",
        spin: "spin 0.8s linear infinite",
      },
    },
  },
  plugins: [],
};
