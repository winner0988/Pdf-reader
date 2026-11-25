// What a form's field values need that other edits do not (B2-09): the value being typed is sent
// when the user leaves its box, so a command that would save or close the document first has to
// leave the box; and every edit gives the document a new id, so two field values sent one right
// after the other cannot both be sent with the id the page was showing.

/** How long to wait for the main process to announce the document an edit made. */
const ANNOUNCE_TIMEOUT_MS = 3000;
const POLL_MS = 5;

/**
 * Resolves once `condition` holds (asked every few milliseconds), or after `timeoutMs` at the
 * latest, so that something lost on the way cannot hold up the edits for good.
 */
export function until(condition: () => boolean, timeoutMs = ANNOUNCE_TIMEOUT_MS): Promise<void> {
  return new Promise((resolve) => {
    const started = Date.now();
    const check = () => {
      if (condition() || Date.now() - started >= timeoutMs) resolve();
      else setTimeout(check, POLL_MS);
    };
    check();
  });
}

/** The values being sent to the document, one after another. */
export class FieldEdits {
  private tail: Promise<void> = Promise.resolve();
  private pending = 0;
  private flushers = new Set<() => Promise<boolean>>();

  /**
   * Registers an active inline editor (e.g. an on-page text box, #719) that must commit before
   * saving or closing the document. Returns an unregister function.
   */
  registerFlusher(flush: () => Promise<boolean>): () => void {
    this.flushers.add(flush);
    return () => {
      this.flushers.delete(flush);
    };
  }

  /** Runs `work` once the edits before it have been answered, whether or not they worked. */
  run<T>(work: () => Promise<T>): Promise<T> {
    this.pending += 1;
    const result = this.tail.then(work);
    this.tail = result.then(
      () => this.answered(),
      () => this.answered(),
    );
    return result;
  }

  private answered() {
    this.pending -= 1;
  }

  /**
   * Calls `next` once the value being typed in a field or inline text box (if one is) has been
   * sent and answered, and with it every earlier one: at once when there is none, so that most
   * commands stay as they are. With `keepFocus` the user, who may go on typing (after saving,
   * say), gets the box back unless they moved on.
   */
  whenSettled(next: () => void, { keepFocus = false } = {}): void {
    const active = document.activeElement;
    const box = active instanceof HTMLElement && active.closest("[data-page-fields]") ? active : null;
    // Leaving the box sends what was typed (its blur handler calls `run`, right now).
    box?.blur();
    const proceed = () => {
      next();
      if (keepFocus && box?.isConnected && document.activeElement === document.body) box.focus();
    };
    const activeFlushers = [...this.flushers];
    if (activeFlushers.length === 0) {
      if (this.pending === 0) proceed();
      else void this.tail.then(proceed);
      return;
    }
    void Promise.all(activeFlushers.map((flush) => flush().catch(() => false))).then((outcomes) => {
      // If an inline editor stayed open due to a validation/font error, do not proceed with save/close.
      if (outcomes.some((ok) => !ok)) return;
      if (this.pending === 0) proceed();
      else void this.tail.then(proceed);
    });
  }
}
