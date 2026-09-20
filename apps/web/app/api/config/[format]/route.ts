import { NextResponse, type NextRequest } from "next/server";
import { requireUser } from "@/lib/auth";
import { subscriptionEntitled } from "@/lib/billing";
import { accessLinks } from "@/lib/vpn-access";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

const FORMATS = new Set(["hiddify", "uri", "singbox", "provision"]);

export async function GET(
  _request: NextRequest,
  context: { params: Promise<{ format: string }> }
) {
  const user = await requireUser();
  const { format } = await context.params;

  if (!FORMATS.has(format)) {
    return NextResponse.json({ error: "unsupported format" }, { status: 404 });
  }

  if (!(await subscriptionEntitled(user.id))) {
    return NextResponse.json({ error: "active subscription required" }, { status: 403 });
  }

  const links = await accessLinks(user.id);
  if (!links) {
    return NextResponse.json({ error: "vpn access is not provisioned" }, { status: 409 });
  }

  const target = links[format as keyof typeof links];
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), 10_000);

  try {
    const upstream = await fetch(target, {
      cache: "no-store",
      signal: controller.signal,
      headers: {
        "User-Agent": "singbox-vpn-web/0.1"
      }
    });

    const body = await upstream.arrayBuffer();
    const headers = new Headers();
    headers.set("Cache-Control", "private, no-store, max-age=0");
    headers.set("Pragma", "no-cache");
    headers.set(
      "Content-Type",
      upstream.headers.get("content-type") || "text/plain; charset=utf-8"
    );

    const extension = format === "singbox" || format === "provision" ? "json" : "txt";
    headers.set("Content-Disposition", `attachment; filename="vpn-${format}.${extension}"`);

    return new NextResponse(body, { status: upstream.status, headers });
  } catch {
    return NextResponse.json({ error: "configuration backend unavailable" }, { status: 502 });
  } finally {
    clearTimeout(timeout);
  }
}
