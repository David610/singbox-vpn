import type { NextRequest } from "next/server";
import { appUrl } from "@/lib/env";

export function assertSameOrigin(request: NextRequest) {
  const origin = request.headers.get("origin");
  if (!origin) return;

  if (new URL(origin).origin !== new URL(appUrl()).origin) {
    throw new Error("Cross-origin request rejected");
  }
}
