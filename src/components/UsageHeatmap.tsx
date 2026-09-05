import { useEffect, useMemo, useRef, useState } from "react";
import { Table2 } from "lucide-react";

import type { UsageDay, UsageRange } from "../ipc/types";

interface UsageHeatmapProps {
  days: UsageDay[];
  range: UsageRange;
}

const MONTHS = [
  "Jan",
  "Feb",
  "Mar",
  "Apr",
  "May",
  "Jun",
  "Jul",
  "Aug",
  "Sep",
  "Oct",
  "Nov",
  "Dec",
];

function dateParts(date: string): [number, number, number] {
  const [year, month, day] = date.split("-").map(Number);
  return [year, month, day];
}

function weekday(date: string): number {
  const [year, month, day] = dateParts(date);
  return new Date(year, month - 1, day).getDay();
}

function activityValue(day: UsageDay): string {
  if (day.activityCount > 0) {
    return `${day.activityCount.toLocaleString()}${day.coverage === "partial" ? " · partial" : ""}`;
  }
  if (day.coverage === "complete") return "0";
  if (day.coverage === "partial") return "— · partial";
  return "Unavailable";
}

function splitValue(day: UsageDay): string {
  const responseLabel = day.responseCount === 1 ? "model response" : "model responses";
  const draftLabel = day.invocationCount === 1 ? "draft run" : "draft runs";
  return `${day.responseCount.toLocaleString()} ${responseLabel} · ${day.invocationCount.toLocaleString()} ${draftLabel}`;
}

function coverageLabel(day: UsageDay): string {
  const base = day.coverage.charAt(0).toUpperCase() + day.coverage.slice(1);
  return day.inProgress ? `${base} · In progress` : base;
}

export function UsageHeatmap({ days, range }: UsageHeatmapProps) {
  const [tableOpen, setTableOpen] = useState(false);
  const [selectedIndex, setSelectedIndex] = useState(Math.max(0, days.length - 1));
  const cells = useRef<Array<HTMLButtonElement | null>>([]);

  useEffect(() => {
    setSelectedIndex(Math.max(0, days.length - 1));
  }, [days]);

  const grid = useMemo(() => {
    if (days.length === 0) return { columns: 0, padded: [] as Array<UsageDay | null> };
    const leading = weekday(days[0].date);
    const columns = Math.ceil((leading + days.length) / 7);
    return {
      columns,
      padded: [
        ...Array<UsageDay | null>(leading).fill(null),
        ...days,
        ...Array<UsageDay | null>(columns * 7 - leading - days.length).fill(null),
      ],
    };
  }, [days]);

  const selected = days[selectedIndex] ?? days[days.length - 1];
  const firstWeekday = days.length > 0 ? weekday(days[0].date) : 0;

  function moveFocus(index: number, offset: number) {
    const next = Math.max(0, Math.min(days.length - 1, index + offset));
    setSelectedIndex(next);
    cells.current[next]?.focus();
  }

  return (
    <section className="usage-card usage-heatmap" aria-labelledby="usage-daily-title">
      <div className="usage-section-heading">
        <div>
          <h2 id="usage-daily-title">Daily activity</h2>
          <p>
            {range.startDate} – {range.endDate} · {range.timeZone}
            {days[days.length - 1]?.inProgress ? " · Today in progress" : ""}
          </p>
        </div>
        <button
          type="button"
          className="usage-compact-button"
          aria-pressed={tableOpen}
          onClick={() => setTableOpen((open) => !open)}
        >
          <Table2 aria-hidden="true" />
          {tableOpen ? "Calendar" : "Daily table"}
        </button>
      </div>

      {tableOpen ? (
        <div className="usage-daily-table-wrap">
          <table className="usage-table usage-daily-table">
            <thead>
              <tr>
                <th>Date</th>
                <th>Recorded activity</th>
                <th>Model responses</th>
                <th>Draft runs</th>
                <th>Coverage</th>
              </tr>
            </thead>
            <tbody>
              {days.map((day) => (
                <tr key={day.date}>
                  <td>
                    {day.date}
                    {day.inProgress ? " · in progress" : ""}
                  </td>
                  <td>{activityValue(day)}</td>
                  <td>{day.responseCount.toLocaleString()}</td>
                  <td>{day.invocationCount.toLocaleString()}</td>
                  <td>{coverageLabel(day)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : (
        <div className="usage-calendar-wrap">
          <div className="usage-weekdays" aria-hidden="true">
            {['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'].map((label) => (
              <span key={label}>{label}</span>
            ))}
          </div>
          <div className="usage-calendar-scroller">
            <div
              className="usage-months"
              aria-hidden="true"
              style={{ gridTemplateColumns: `repeat(${grid.columns}, 22px)` }}
            >
              {Array.from({ length: grid.columns }, (_, column) => {
                const dayIndex = Math.max(0, column * 7 - firstWeekday);
                const current = days[Math.min(dayIndex, Math.max(0, days.length - 1))];
                const previousIndex = Math.max(0, (column - 1) * 7 - firstWeekday);
                const previous = days[Math.min(previousIndex, Math.max(0, days.length - 1))];
                const [, month] = current ? dateParts(current.date) : [0, 0, 0];
                const [, previousMonth] = previous ? dateParts(previous.date) : [0, 0, 0];
                const [, endingMonth] = days.length > 0
                  ? dateParts(days[days.length - 1].date)
                  : [0, 0, 0];
                return (
                  <span key={column}>
                    {column === grid.columns - 1
                      ? MONTHS[endingMonth - 1]
                      : column === 0 || month !== previousMonth
                        ? MONTHS[month - 1]
                        : ""}
                  </span>
                );
              })}
            </div>
            <div
              className="usage-calendar-grid"
              role="group"
              aria-label="Daily recorded activity"
              style={{ gridTemplateColumns: `repeat(${grid.columns}, 16px)` }}
            >
              {grid.padded.map((day, paddedIndex) => {
                if (!day) {
                  return <span key={`outside-${paddedIndex}`} aria-hidden="true" />;
                }
                const index = paddedIndex - firstWeekday;
                const unknown = day.coverage === "none" || (day.coverage === "partial" && day.activityCount === 0);
                const label = `${day.date}, ${activityValue(day)} recorded activities, ${splitValue(day)}, ${coverageLabel(day)}`;
                return (
                  <button
                    key={day.date}
                    ref={(element) => {
                      cells.current[index] = element;
                    }}
                    type="button"
                    className={`usage-calendar-cell usage-heat-${Math.min(day.activityCount, 4)}${unknown ? " is-unknown" : ""}${day.coverage === "partial" ? " is-partial" : ""}`}
                    tabIndex={index === selectedIndex ? 0 : -1}
                    title={label}
                    aria-label={label}
                    onFocus={() => setSelectedIndex(index)}
                    onMouseEnter={() => setSelectedIndex(index)}
                    onClick={() => setSelectedIndex(index)}
                    onKeyDown={(event) => {
                      const offset =
                        event.key === "ArrowRight"
                          ? 7
                          : event.key === "ArrowLeft"
                            ? -7
                            : event.key === "ArrowDown"
                              ? 1
                              : event.key === "ArrowUp"
                                ? -1
                                : 0;
                      if (offset === 0) return;
                      event.preventDefault();
                      event.stopPropagation();
                      moveFocus(index, offset);
                    }}
                  />
                );
              })}
            </div>
          </div>
          <div className="usage-history-note">
            <strong>History begins with collection</strong>
            <span>Earlier dates and observation gaps are unknown. A solid empty cell is a verified zero.</span>
          </div>
        </div>
      )}

      {selected && (
        <output className="usage-day-detail" aria-live="polite">
          {selected.date} · {activityValue(selected)} recorded activities · {splitValue(selected)} · Coverage: {coverageLabel(selected)}
        </output>
      )}

      <div className="usage-legend" aria-label="Recorded activity intensity legend">
        <span>Recorded activity / day</span>
        {[0, 1, 2, 3, 4].map((count) => (
          <span key={count}>
            <i className={`usage-calendar-cell usage-heat-${count}`} aria-hidden="true" />
            {count === 4 ? "4+" : count}
          </span>
        ))}
        <span>
          <i className="usage-calendar-cell is-unknown" aria-hidden="true" />
          Unknown
        </span>
        <span>
          <i className="usage-calendar-cell is-partial" aria-hidden="true" />
          Partial
        </span>
      </div>
    </section>
  );
}
