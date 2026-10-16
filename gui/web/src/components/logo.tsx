/** Runa logo mark (assets/logo/logo-small.svg, tuned for 16-32px). */
export function Logo({ size = 22 }: { size?: number }) {
  return (
    <svg aria-hidden width={size} height={size} viewBox="0 0 32 32" className="flex-none">
      <defs>
        <linearGradient id="runa-logo-bg" x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stopColor="#6366F1" />
          <stop offset="1" stopColor="#8B5CF6" />
        </linearGradient>
      </defs>
      <rect width="32" height="32" rx="7" fill="url(#runa-logo-bg)" />
      <path d="M11 8v15c0-6 10-5 10-11" fill="none" stroke="#fff" strokeWidth="3.6" strokeLinecap="round" strokeLinejoin="round" />
      <circle cx="11" cy="24.5" r="3.2" fill="#fff" />
      <circle cx="21" cy="10.5" r="3.8" fill="#fff" />
    </svg>
  );
}
