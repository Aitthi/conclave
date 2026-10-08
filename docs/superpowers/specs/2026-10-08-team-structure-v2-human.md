# สรุปการจัดทีม CLI Agents (v2) — human spec, received 2026-10-08

Verbatim requirement from the human (Thai). The plan that implements it is
`docs/plans/2026-10-08-team-tiers-v2.md`; the decisions are ADR 0010 Amendment 3.

## 1. โครงสร้างทีมปัจจุบัน

```
Human — ผู้ตัดสินสุดท้าย
└── Detoro — Lead (Fable 5.1)
    └── Aitthi — Coordinator (Sonnet 5.5)
        ├── Arta     — Designer (Opus 5.5)
        ├── Dew      — Implementer (Complex) (Opus 5.5)
        ├── knock2   — Implementer (Complex) (Sonnet 5.5)   ← แนะนำเปลี่ยนเป็น Standard
        ├── Tiësto   — Implementer (Routine) (Sonnet 5.5)
        ├── Zedd     — Implementer (Routine) (Sonnet 5.5)
        ├── Guetta   — Researcher (Sonnet 5.5)
        ├── Mellow   — Reviewer (Opus 5.5)
        └── Illenium — Runner (Haiku)
```

## 2. เพิ่มระดับ Implementer (Standard)

ใช้ Sonnet ได้เร็ว อยู่ระหว่าง Routine กับ Complex ตำแหน่งนี้ตรงกับ **knock2** อยู่แล้ว (Sonnet, ระดับ Mid) จึงแนะนำให้เปลี่ยนชื่อ role จาก Complex เป็น Standard

แบ่งระดับตาม **ขอบเขตการตัดสินใจ** ไม่ใช่ความยากของโค้ด เพื่อให้ Coordinator จ่ายงานได้ง่าย

| ระดับ | ขอบเขต | ตัวอย่างงาน | โมเดล | Agent |
|---|---|---|---|---|
| Routine | ทำตาม pattern ที่มีอยู่ ไม่ตัดสินใจเอง | เพิ่ม field, CRUD ตาม template, เขียนเทสต์ตาม spec, แก้ bug ที่ระบุจุดชัด | Sonnet | Tiësto, Zedd |
| Standard | ทำฟีเจอร์ครบในโมดูลเดียว ตัดสินใจภายในได้ แต่ไม่แตะ interface กลาง | endpoint ใหม่พร้อม validation และเทสต์, component ที่มี state, เชื่อม API ภายนอกตาม spec, refactor ภายในโมดูล | Sonnet | knock2 |
| Complex | ข้ามหลายโมดูล หรือต้องสร้าง pattern ใหม่ | refactor ใหญ่, เปลี่ยน data model, concurrency, performance, security, spec ที่ยังกำกวม | Opus | Dew |

### กติกาของ Standard

1. **ห้ามแก้ interface ที่โมดูลอื่นใช้** (API contract, shared types, schema) ถ้าจำเป็นให้หยุดและรายงาน `needs_decision` เพื่อย้ายงานไป Complex
2. **Lead ติด tag ระดับความยากในแผน** Coordinator จ่ายงานตาม tag โดยไม่ประเมินเอง
3. **จำกัดขนาดงานต่อ task** ไม่เกิน 1 โมดูล งานเล็กทำเร็วและรีวิวง่าย

ถ้างานไปกองที่ Dew จนรอคิวบ่อย ค่อยพิจารณาเพิ่ม Complex อีกหนึ่งตัว

## 3. แก้ปัญหา Reviewer ทำงานไม่ทัน

**สาเหตุ:** Implementer 4 ตัวและ Designer ส่งงานเข้า Mellow (Opus) คนเดียว

ทางแก้สามชั้น เรียงจากผลมากไปน้อย:

### 3.1 ให้ Runner ตรวจก่อนถึง Reviewer

Illenium (Haiku) รัน lint, typecheck, build และเทสต์ก่อน ไม่ผ่านส่งกลับ Implementer ทันที Reviewer จะไม่เสียเวลากับงานที่ยังพัง

### 3.2 แบ่ง Reviewer ตามระดับงาน

เพิ่ม **Reviewer (Standard)** ที่ใช้ Sonnet อีกหนึ่งตัว

| ระดับงาน | ผู้รีวิว | โมเดล |
|---|---|---|
| Routine | Reviewer (Standard) — ใหม่ | Sonnet |
| Standard | Reviewer (Standard) — ใหม่ | Sonnet |
| Complex, interface กลาง, security | Mellow | Opus |
| งาน design จาก Arta | Mellow | Opus |

### 3.3 ลดภาระต่องานรีวิว

- **Implementer แนบสรุปมากับงาน** เปลี่ยนอะไร ทำไม และจุดที่ไม่แน่ใจ
- **Reviewer ดูแค่ diff** พร้อม checklist ตามระดับงาน
- **Reviewer ให้ความเห็นอย่างเดียว ไม่แก้โค้ดเอง**

### ข้อควรระวัง

Sonnet รีวิวงานที่ Sonnet เขียน อาจมองข้ามจุดบอดแบบเดียวกัน ชดเชยด้วย:

- พึ่งเทสต์อัตโนมัติจาก Runner ให้มากขึ้น
- Mellow สุ่มตรวจงานที่ Reviewer (Standard) ผ่านแล้วตอนจบ milestone
- งานประเภทไหนหลุดบ่อย ให้ย้ายขึ้นไปให้ Mellow รีวิว

> ลำดับการลงมือ: ทำข้อ 3.1 และ 3.3 ก่อน ถ้ายังไม่ทันค่อยเพิ่ม Reviewer ตามข้อ 3.2

## 4. Flow การทำงาน

```
Lead เขียนแผน (ติด tag: routine / standard / complex)
    ↓
Coordinator จ่ายงานตาม tag + dependency
    ↓
Implementer ทำเสร็จ (แนบสรุป) → Coordinator
    ↓
Runner (Haiku): lint / typecheck / build / test
    ├─ ไม่ผ่าน → ส่งกลับ Implementer
    └─ ผ่าน → แยกตาม tag
         ├─ Routine / Standard → Reviewer (Standard, Sonnet)
         └─ Complex / interface / security / design → Mellow (Opus)
              ├─ ไม่ผ่าน → ส่งกลับ Implementer (สูงสุด 2 รอบ แล้ว escalate ไป Lead)
              └─ ผ่าน → Coordinator อัปเดตสถานะ → จ่ายงานถัดไป
    ↓
จบ milestone → Lead รีวิวภาพรวม + Mellow สุ่มตรวจงาน Standard
```

## 5. ทีมหลังปรับ

| Agent | ตำแหน่ง | โมเดล | การเปลี่ยนแปลง |
|---|---|---|---|
| Detoro | Lead | Fable 5.1 | — |
| Aitthi | Coordinator | Sonnet 5.5 | เพิ่มการจ่ายงานผ่าน Runner และแยก Reviewer ตาม tag |
| Arta | Designer | Opus 5.5 | — |
| Dew | Implementer (Complex) | Opus 5.5 | — |
| knock2 | Implementer (Standard) | Sonnet 5.5 | เปลี่ยน role จาก Complex |
| Tiësto | Implementer (Routine) | Sonnet 5.5 | — |
| Zedd | Implementer (Routine) | Sonnet 5.5 | — |
| Guetta | Researcher | Sonnet 5.5 | — |
| Mellow | Reviewer (Complex) | Opus 5.5 | รับเฉพาะ Complex / interface / security / design + สุ่มตรวจ |
| *(ใหม่)* | Reviewer (Standard) | Sonnet 5.5 | รีวิว Routine และ Standard |
| Illenium | Runner | Haiku | ตรวจอัตโนมัติก่อนส่งรีวิว |

Instruction that closed the message: "เพิ่ม ตำแหน่ง" (add the positions).
