/** @type {import('tailwindcss').Config} */
export default {
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  theme: {
    extend: {
      colors: {
        studio: {
          bg: '#0b0e14',
          surface: '#111622',
          panel: '#161c2c',
          elevated: '#1d2538',
          border: '#263047',
          muted: '#7f8ea8',
          text: '#e6edf7',
        },
        signal: {
          control: '#f59e0b',   // Amber = Control Float
          midi: '#22d3ee',      // Cyan = MIDI / Trigger
          video: '#a78bfa',     // Violet = Video / NDI
          sensor: '#34d399',    // Emerald = Hardware / Sensor
          bytes: '#94a3b8',     // Slate = ByteStream
        },
      },
      fontFamily: {
        mono: ['ui-monospace', 'SFMono-Regular', 'Menlo', 'Monaco', 'Consolas', 'monospace'],
      },
    },
  },
  plugins: [],
};
