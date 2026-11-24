#!/usr/bin/env python3
"""Banco simulado para el caso de uso «Pago de tarjeta desde una cuenta».

Cuatro servicios de dos sistemas, como en una integración real:
  Sistema de cuentas (origen del dinero)
    GET  /cuentas/v1/cuentas/{nroCuenta}                 consulta saldo y estado
    POST /cuentas/v1/cuentas/{nroCuenta}/retenciones     retiene (congela) el monto
  Core de tarjetas de crédito (destino)
    POST /core-tarjetas/v2/pagos                         abona a la tarjeta usando la retención
    GET  /core-tarjetas/v2/transacciones/{transaccionId} detalle y estado contable

Uso: python3 ejemplos/banco/simulador.py [puerto]   (por defecto 4500; sólo escucha en 127.0.0.1)
"""
import json, sys, uuid, datetime
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

CUENTAS = {
    "191-0001": {"titular": "Ana Quispe", "moneda": "PEN", "saldoDisponible": 5000.00, "estado": "ACTIVA"},
    "191-0002": {"titular": "Luis Huamán", "moneda": "PEN", "saldoDisponible": 80.50, "estado": "ACTIVA"},
    "191-0003": {"titular": "Rosa Flores", "moneda": "PEN", "saldoDisponible": 9000.00, "estado": "BLOQUEADA"},
}
TARJETAS = {"4111111111111111": {"titular": "Ana Quispe", "deudaActual": 3200.00}}
LIMITE_POSTEO_INMEDIATO = 1000.00  # pagos mayores pasan a revisión antifraude: quedan PENDING
RETENCIONES, TRANSACCIONES = {}, {}


def ahora():
    return datetime.datetime.now().astimezone().isoformat(timespec="seconds")


def enmascarar(nro):
    return f"{nro[:4]} **** **** {nro[-4:]}"


def error(status, codigo, mensaje):
    return status, {"codigo": codigo, "mensaje": mensaje}


def consultar_cuenta(nro, _body):
    c = CUENTAS.get(nro)
    if not c:
        return error(404, "CUENTA_NO_EXISTE", f"No hay cuenta {nro}")
    return 200, {"nroCuenta": nro, **c}


def retener(nro, body):
    c = CUENTAS.get(nro)
    if not c:
        return error(404, "CUENTA_NO_EXISTE", f"No hay cuenta {nro}")
    if c["estado"] != "ACTIVA":
        return error(423, "CUENTA_BLOQUEADA", "La cuenta no admite débitos")
    monto = float(body.get("monto", 0))
    if monto <= 0:
        return error(400, "MONTO_INVALIDO", "El monto debe ser mayor que 0")
    if monto > c["saldoDisponible"]:
        return error(422, "SALDO_INSUFICIENTE", f"Disponible {c['saldoDisponible']:.2f}, pedido {monto:.2f}")
    rid = "RET-" + uuid.uuid4().hex[:8].upper()
    RETENCIONES[rid] = {"nroCuenta": nro, "monto": monto, "moneda": c["moneda"], "usada": False}
    # shortcut: el saldo guardado no baja para que los casos QA se puedan repetir; un banco real lo descuenta.
    return 201, {"retencionId": rid, "monto": monto, "saldoDisponible": round(c["saldoDisponible"] - monto, 2),
                 "venceEn": "PT15M", "referencia": body.get("referencia", "")}


def pagar_tarjeta(_, body):
    t = TARJETAS.get(body.get("nroTarjeta", ""))
    if not t:
        return error(404, "TARJETA_NO_EXISTE", "La tarjeta no está en el core")
    r = RETENCIONES.get(body.get("retencionId", ""))
    if not r or r["usada"]:
        return error(409, "RETENCION_INVALIDA", "La retención no existe o ya se usó")
    r["usada"] = True
    pendiente = r["monto"] > LIMITE_POSTEO_INMEDIATO
    tid = "TX-" + uuid.uuid4().hex[:10].upper()
    TRANSACCIONES[tid] = {
        "transaccionId": tid,
        "estado": "PENDING" if pendiente else "POSTED",
        "motivo": "REVISION_ANTIFRAUDE" if pendiente else None,
        "codigoAutorizacion": None if pendiente else uuid.uuid4().hex[:6].upper(),
        "fechaContable": None if pendiente else datetime.date.today().isoformat(),
        "monto": r["monto"], "moneda": r["moneda"],
        "tarjeta": enmascarar(body["nroTarjeta"]),
        "cuentaOrigen": r["nroCuenta"], "retencionId": body["retencionId"],
        "registrada": ahora(),
    }
    return 202, {"transaccionId": tid, "estado": "RECEIVED"}


def detalle(tid, _body):
    t = TRANSACCIONES.get(tid)
    return (200, t) if t else error(404, "TRANSACCION_NO_EXISTE", f"No hay transacción {tid}")


RUTAS = [  # (método, prefijo, sufijo, función): el tramo del medio es el id
    ("GET", "/cuentas/v1/cuentas/", "", consultar_cuenta),
    ("POST", "/cuentas/v1/cuentas/", "/retenciones", retener),
    ("POST", "/core-tarjetas/v2/pagos", "", pagar_tarjeta),
    ("GET", "/core-tarjetas/v2/transacciones/", "", detalle),
]


class Banco(BaseHTTPRequestHandler):
    def atender(self, metodo):
        largo = int(self.headers.get("Content-Length") or 0)
        try:
            body = json.loads(self.rfile.read(largo) or b"{}")
        except json.JSONDecodeError:
            return self.responder(*error(400, "JSON_INVALIDO", "El body no es JSON"))
        for m, pre, suf, fn in RUTAS:
            if m == metodo and self.path.startswith(pre) and self.path.endswith(suf):
                ident = self.path[len(pre):len(self.path) - len(suf)].strip("/")
                if (ident == "") == (fn is pagar_tarjeta):  # sólo /pagos va sin id
                    return self.responder(*fn(ident, body))
        self.responder(*error(404, "RUTA_NO_EXISTE", f"{metodo} {self.path}"))

    def responder(self, status, data):
        raw = json.dumps(data, ensure_ascii=False).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("X-Request-Id", self.headers.get("X-Request-Id") or uuid.uuid4().hex)
        self.send_header("Content-Length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def do_GET(self):
        self.atender("GET")

    def do_POST(self):
        self.atender("POST")


if __name__ == "__main__":
    puerto = int(sys.argv[1]) if len(sys.argv) > 1 else 4500
    print(f"Banco simulado en http://127.0.0.1:{puerto}")
    ThreadingHTTPServer(("127.0.0.1", puerto), Banco).serve_forever()
