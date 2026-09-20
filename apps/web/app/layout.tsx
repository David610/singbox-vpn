import type { Metadata } from "next";
import Link from "next/link";
import "./globals.css";

const productName = process.env.NEXT_PUBLIC_PRODUCT_NAME || "VPN";

export const metadata: Metadata = {
  title: {
    default: productName,
    template: `%s · ${productName}`
  },
  description: "Purchase VPN access and import a configuration into a compatible client.",
  robots: {
    index: true,
    follow: true
  }
};

export default function RootLayout({ children }: Readonly<{ children: React.ReactNode }>) {
  return (
    <html lang="en">
      <body>
        <div className="shell">
          <header className="site-header">
            <Link className="brand" href="/">{productName}</Link>
            <nav className="nav" aria-label="Main navigation">
              <Link href="/auth/login">Log in</Link>
              <Link className="button" href="/auth/signup">Get access</Link>
            </nav>
          </header>
          <main>{children}</main>
          <footer className="footer">
            <span>© {new Date().getFullYear()} {productName}</span>
            <nav aria-label="Legal">
              <Link href="/privacy">Privacy</Link>
              <Link href="/terms">Terms</Link>
              <Link href="/legal">Legal notice</Link>
            </nav>
          </footer>
        </div>
      </body>
    </html>
  );
}
