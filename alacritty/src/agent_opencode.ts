import { spawn } from "node:child_process"
import { fileURLToPath } from "node:url"

type AgentState = "ready" | "working" | "input" | "permission" | "done" | "error" | "unknown"
type Session = { parentID?: string }
type Event = { type: string; properties?: { sessionID?: string } }
type Api = {
  route: { current: { name: string; params?: { sessionID?: string } } }
  state: {
    ready: boolean
    session: {
      get(id: string): Session | undefined
      status(id: string): { type: string } | undefined
      permission(id: string): readonly unknown[]
      question(id: string): readonly unknown[]
      family?(id: string): readonly string[]
    }
  }
  event: { on(type: string, callback: (event: Event) => void): (() => void) | undefined }
  lifecycle?: { onDispose(callback: () => void): unknown }
}

type Context = {
  location?: unknown
  ui: { router: { current(): { type: string; sessionID?: string } } }
  data: {
    on(type: string, callback: (event: { type: string; data?: { sessionID?: string } }) => void): () => void
    session: {
      root(id: string): string
      family(id: string): string[]
      status(id: string): "idle" | "running"
      permission: { list(id: string): readonly unknown[] | undefined }
      form: { list(id: string, location?: unknown): readonly { status?: string }[] | undefined }
    }
  }
}

const plugin = {
  id: "termust-agent-status",
  setup(context: Context) {
    let dispose: (() => void) | undefined
    void plugin.tui({
      route: { get current() {
        const route = context.ui.router.current()
        return { name: route.type, params: { sessionID: route.sessionID } }
      } },
      state: {
        ready: true,
        session: {
          get(id) {
            const parent = context.data.session.root(id)
            return parent === id ? {} : { parentID: parent }
          },
          family(id) { return context.data.session.family(id).slice(0, 256) },
          status(id) { return { type: context.data.session.status(id) === "running" ? "busy" : "idle" } },
          permission(id) { return context.data.session.permission.list(id) ?? [] },
          question(id) { return context.data.session.form.list(id, context.location) ?? [] },
        },
      },
      event: { on(type, callback) {
        return context.data.on(type, (event) => callback({ type: event.type, properties: event.data }))
      } },
      lifecycle: { onDispose(callback) { dispose = callback } },
    })
    return () => dispose?.()
  },
  tui(api: Api) {
    if (!process.env.TERMUST_AGENT_DIR || !process.env.TERMUST_AGENT_RUN || !process.env.TERMUST_AGENT_PID) return
    const bridge = process.env.TERMUST_AGENT_BRIDGE || fileURLToPath(new URL("agent_bridge.py", import.meta.url))
    const python = process.env.TERMUST_AGENT_PYTHON || "python3"
    let previous = ""
    let queued: { state: AgentState; session: string; limited: boolean } | undefined
    let sending = false
    let disposed = false
    const worked = new Set<string>()
    const failed = new Set<string>()
    const cancelled = new Set<string>()
    const known = new Set<string>()
    const remember = (values: Set<string>, id: string) => {
      if (!values.has(id) && values.size >= 256) values.delete(values.values().next().value!)
      values.add(id)
    }

    const deliver = async () => {
      if (sending) return
      sending = true
      while (queued && !disposed) {
        const signal = queued
        queued = undefined
        await new Promise<void>((resolve) => {
          const child = spawn(python, [bridge, "hook", "opencode", signal.state], {
            stdio: ["pipe", "ignore", "ignore"],
            env: process.env,
          })
          const timer = setTimeout(() => child.kill("SIGTERM"), 1500)
          const finish = () => { clearTimeout(timer); resolve() }
          child.once("error", finish)
          child.once("exit", finish)
          child.stdin.on("error", () => {})
          child.stdin.end(JSON.stringify({ session_id: signal.session, limited: signal.limited }))
        })
      }
      sending = false
    }

    const emit = (state: AgentState, session: string, limited = false) => {
      const key = `${session}:${state}:${limited}`
      if (previous === key || disposed) return
      previous = key
      queued = { state, session, limited }
      void deliver()
    }

    const root = (id: string) => {
      const seen = new Set<string>()
      while (id && seen.size < 16 && !seen.has(id)) {
        seen.add(id)
        const session = api.state.session.get(id)
        if (!session) return undefined
        if (!session.parentID) return id
        id = session.parentID
      }
      return undefined
    }

    const refresh = () => {
      if (disposed) return
      try {
        if (!api.state.ready) return
        const route = api.route.current
        if (route.name !== "session" || !route.params?.sessionID) {
          emit("ready", "")
          return
        }
        const id = root(route.params.sessionID)
        if (!id) { emit("unknown", "", true); return }
        const related = api.state.session.family?.(id) ?? Array.from(known).filter((other) => other !== id && root(other) === id)
        const family = Array.from(new Set([id, ...related])).slice(0, 256)
        if (family.some((session) => api.state.session.permission(session).length > 0)) {
          emit("permission", id)
          return
        }
        if (family.some((session) => api.state.session.question(session).length > 0)) {
          emit("input", id)
          return
        }
        const statuses = family.map((session) => api.state.session.status(session)?.type)
        const status = api.state.session.status(id)?.type
        if (failed.has(id)) {
          emit("error", id)
        } else if (cancelled.has(id)) {
          emit("ready", id)
        } else if (statuses.some((value) => value === "busy" || value === "retry")) {
          remember(worked, id)
          emit("working", id)
        } else if (status === "idle" || status === undefined) {
          emit(worked.has(id) ? "done" : "ready", id)
        } else {
          emit("unknown", id, true)
        }
      } catch {
        emit("unknown", "", true)
      }
    }

    const events = ["session.status", "session.idle", "session.error", "permission.asked", "permission.replied",
      "question.asked", "question.replied", "question.rejected", "permission.v2.asked", "permission.v2.replied",
      "question.v2.asked", "question.v2.replied", "question.v2.rejected", "form.created", "form.replied", "form.cancelled",
      "session.execution.started", "session.execution.succeeded", "session.execution.failed", "session.execution.interrupted"]
    const subscriptions = events.map((type) => api.event.on(type, (event) => {
      const id = event.properties?.sessionID
      if (id) {
        remember(known, id)
        if (event.type === "session.error" || event.type === "session.execution.failed") remember(failed, id)
        if (event.type === "session.execution.interrupted") remember(cancelled, id)
        if (event.type === "session.execution.started" || (event.type === "session.status" && api.state.session.status(id)?.type === "busy")) {
          failed.delete(id)
          cancelled.delete(id)
          remember(worked, id)
        }
      }
      queueMicrotask(refresh)
    }))
    const timer = setInterval(refresh, 500)
    timer.unref?.()
    api.lifecycle?.onDispose(() => {
      disposed = true
      clearInterval(timer)
      for (const unsubscribe of subscriptions) unsubscribe?.()
    })
    refresh()
  },
}

export default plugin
