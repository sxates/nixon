/** @type {import('tailwindcss').Config} */
module.exports = {
    darkMode: ['class'],
    content: [
    './src/pages/**/*.{js,ts,jsx,tsx,mdx}',
    './src/components/**/*.{js,ts,jsx,tsx,mdx}',
    './src/app/**/*.{js,ts,jsx,tsx,mdx}',
    // src/lib holds class-name literals (e.g. speaker-colors.ts → bg-chart-1,
    // text-chart-3). Without this they're purged and speaker avatars/labels lose color.
    './src/lib/**/*.{js,ts,jsx,tsx,mdx}',
  ],
  theme: {
  	extend: {
  		fontFamily: {
  			sans: ['var(--font-archivo)', '"Helvetica Neue"', '-apple-system', 'sans-serif'],
  			narrow: ['var(--font-archivo-narrow)', '"Arial Narrow"', 'sans-serif'],
  			reading: ['var(--font-plex-sans)', '"Helvetica Neue"', 'sans-serif'],
  			mono: ['var(--font-plex-mono)', 'Menlo', 'monospace'],
  			type: ['var(--font-courier-prime)', '"Courier New"', 'Courier', 'monospace'],
  		},
  		colors: {
  			background: 'hsl(var(--background))',
  			foreground: 'hsl(var(--foreground))',
  			border: 'hsl(var(--border))',
  			input: 'hsl(var(--input))',
  			ring: 'hsl(var(--ring))',
  			primary: {
  				DEFAULT: 'hsl(var(--primary))',
  				foreground: 'hsl(var(--primary-foreground))'
  			},
  			secondary: {
  				DEFAULT: 'hsl(var(--secondary))',
  				foreground: 'hsl(var(--secondary-foreground))'
  			},
  			brand: {
  				DEFAULT: 'hsl(var(--brand))',
  				foreground: 'hsl(var(--brand-foreground))'
  			},
  			record: {
  				DEFAULT: 'hsl(var(--record))',
  				foreground: 'hsl(var(--record-foreground))',
  				ink: 'hsl(var(--record-ink))'
  			},
  			card: {
  				DEFAULT: 'hsl(var(--card))',
  				foreground: 'hsl(var(--card-foreground))'
  			},
  			popover: {
  				DEFAULT: 'hsl(var(--popover))',
  				foreground: 'hsl(var(--popover-foreground))'
  			},
  			muted: {
  				DEFAULT: 'hsl(var(--muted))',
  				foreground: 'hsl(var(--muted-foreground))'
  			},
  			accent: {
  				DEFAULT: 'hsl(var(--accent))',
  				foreground: 'hsl(var(--accent-foreground))'
  			},
  			destructive: {
  				DEFAULT: 'hsl(var(--destructive))',
  				foreground: 'hsl(var(--destructive-foreground))'
  			},
  			chart: {
  				'1': 'hsl(var(--chart-1))',
  				'2': 'hsl(var(--chart-2))',
  				'3': 'hsl(var(--chart-3))',
  				'4': 'hsl(var(--chart-4))',
  				'5': 'hsl(var(--chart-5))',
  				'6': 'hsl(var(--chart-6))',
  				'7': 'hsl(var(--chart-7))',
  				'8': 'hsl(var(--chart-8))'
  			},
  			success: {
  				DEFAULT: 'hsl(var(--success))',
  				foreground: 'hsl(var(--success-foreground))'
  			},
  			panel: 'hsl(var(--panel))',
  			engrave: 'hsl(var(--engrave))',
  			'lamp-amber': 'hsl(var(--lamp-amber))',
  			'lamp-red': 'hsl(var(--lamp-red))',
  			'lamp-ink': 'hsl(var(--lamp-ink))',
  			paper: {
  				DEFAULT: 'hsl(var(--paper))',
  				ink: 'hsl(var(--paper-ink))'
  			},
  			key: 'hsl(var(--key))',
  			well: 'hsl(var(--well))',
  			walnut: 'hsl(var(--walnut))',
  			'meter-over': 'hsl(var(--meter-over))'
  		},
  		borderRadius: {
  			lg: 'var(--radius)',
  			md: 'calc(var(--radius) - 1px)',
  			sm: 'calc(var(--radius) - 2px)'
  		},
  		keyframes: {
  			'accordion-down': {
  				from: {
  					height: '0'
  				},
  				to: {
  					height: 'var(--radix-accordion-content-height)'
  				}
  			},
  			'accordion-up': {
  				from: {
  					height: 'var(--radix-accordion-content-height)'
  				},
  				to: {
  					height: '0'
  				}
  			},
  			// specs/0057 §3.4 — the reel hubs. Named animations (not arbitrary
  			// `duration-[…]`) because tailwindcss-animate makes arbitrary duration/ease
  			// values ambiguous, so Tailwind emits nothing for them.
  			reel: {
  				from: {
  					transform: 'rotate(0deg)'
  				},
  				to: {
  					transform: 'rotate(360deg)'
  				}
  			}
  		},
  		animation: {
  			'accordion-down': 'accordion-down 0.2s ease-out',
  			'accordion-up': 'accordion-up 0.2s ease-out',
  			// Owner feedback (0.1.0): slower, and the two hubs turn at different rates like a
  			// real deck — the supply reel (left, fuller) lags the take-up reel (right).
  			reel: 'reel 2.6s linear infinite',
  			'reel-slow': 'reel 3.6s linear infinite',
  			'reel-spindown': 'reel 0.9s ease-out 1'
  		}
  	}
  },
  // Union of the plugin sets from the former tailwind.config.js/.ts duel (spec 0031):
  // Tailwind resolves .js before .ts, so this file's theme was always the live one;
  // typography is carried over from the retired .ts config (no `prose` usage in src today).
  plugins: [require("tailwindcss-animate"), require("@tailwindcss/typography")],
}