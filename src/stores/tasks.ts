// 批量任务状态（全局保存，切换页面不丢失）。由 App 中的全局 `job` 事件监听驱动。
import { defineStore } from "pinia";
import { reactive } from "vue";

import { baseName, type JobEvent, type Report } from "../api/app";

export type FileStatus = "pending" | "running" | "done" | "failed" | "cancelled";

export interface FileItem {
  path: string;
  name: string;
  status: FileStatus;
  /** 在线解析阶段（uploading / pending / running / converting / downloading） */
  stage?: string;
  /** 文档：选中页数 / 需识别页数 / 已完成页数 */
  total?: number;
  toOcr?: number;
  done: number;
  error?: string;
  outputs: string[];
  elapsedMs?: number;
  text?: string;
  blocks?: number;
  report?: Report;
}

export interface TaskState {
  files: FileItem[];
  jobId: number | null;
  running: boolean;
  startedAt?: number;
  finishedAt?: number;
  outputs: string[];
  error?: string;
  cancelled?: boolean;
}

const newTask = (): TaskState => ({ files: [], jobId: null, running: false, outputs: [] });

export const useTasks = defineStore("tasks", () => {
  const doc = reactive<TaskState>(newTask());
  const batch = reactive<TaskState>(newTask());

  function addFiles(t: TaskState, paths: string[]) {
    const known = new Set(t.files.map((f) => f.path));
    for (const p of paths) {
      if (known.has(p)) continue;
      known.add(p);
      t.files.push({ path: p, name: baseName(p), status: "pending", done: 0, outputs: [] });
    }
  }

  function begin(t: TaskState, job: number) {
    t.jobId = job;
    t.running = true;
    t.startedAt = Date.now();
    t.finishedAt = undefined;
    t.outputs = [];
    t.error = undefined;
    t.cancelled = false;
    for (const f of t.files) {
      Object.assign(f, {
        status: "pending",
        done: 0,
        total: undefined,
        toOcr: undefined,
        stage: undefined,
        error: undefined,
        outputs: [],
        text: undefined,
        report: undefined,
      });
    }
  }

  function handle(e: JobEvent) {
    const t = doc.jobId === e.job ? doc : batch.jobId === e.job ? batch : null;
    if (!t) return;
    if (e.kind === "job_done") {
      t.running = false;
      t.finishedAt = Date.now();
      t.outputs = e.outputs;
      t.error = e.error ?? undefined;
      t.cancelled = e.cancelled;
      // 未完成的文件：取消则标记取消；否则（如引擎加载失败）标记失败
      for (const f of t.files) {
        if (f.status === "pending" || f.status === "running") {
          f.status = e.cancelled ? "cancelled" : "failed";
          if (!e.cancelled) f.error ??= e.error ?? undefined;
        }
      }
      return;
    }
    const f = t.files[e.file];
    if (!f) return;
    switch (e.kind) {
      case "file_start":
        f.status = "running";
        break;
      case "planned":
        f.total = e.total;
        f.toOcr = e.toOcr;
        break;
      case "page":
        f.done += 1;
        break;
      case "progress":
        f.stage = e.stage;
        if (e.total) {
          f.toOcr = e.total;
          f.total = e.total;
          f.done = e.done;
        }
        break;
      case "file_done":
        f.status = e.ok ? "done" : "failed";
        f.error = e.error ?? undefined;
        f.elapsedMs = e.elapsedMs;
        f.outputs = e.outputs;
        f.report = e.report ?? undefined;
        f.text = e.text ?? undefined;
        f.blocks = e.blocks;
        break;
    }
  }

  return { doc, batch, addFiles, begin, handle };
});
