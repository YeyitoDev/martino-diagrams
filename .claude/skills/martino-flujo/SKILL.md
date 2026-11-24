---
name: martino-flujo
description: Crea un flujo completo de Martino en YAML, lo inserta en la carpeta de trabajo y lo ejecuta con sus casos QA hasta que pasen. Úsala cuando pidan "crea un flujo", "arma un flujo en Martino", "automatiza estas llamadas como flujo", "pasa este proceso a Martino" o cuando un agente deba orquestar servicios (llamadas HTTP, condiciones, tablas, cifrado) dentro de Martino.
---

# Crear, insertar y ejecutar un flujo de Martino

Un flujo es un grafo de pasos (entrada → servicios → decisiones → respuesta). Lo escribes en YAML, la
CLI lo valida y lo guarda en `flows/<id>.flow.json` (la UI lo ve al refrescar), y luego lo ejecutas
con sus casos QA. No termines hasta que `martino flow <id> --cases` salga con código 0.

## 0. Ubica Martino

Si tienes las herramientas MCP `martino_*` (servidor `martino mcp`, registrado en `.mcp.json`), úsalas en vez de la CLI:
`martino_listar`, `martino_peticion`, `martino_flujo` (= `flow-yaml`), `martino_importar_flujo` (= `flow-import`),
`martino_ejecutar_flujo` (= `flow`, con `detalle: true` ves lo que respondió cada paso) `martino_enviar` (envía un cURL o una petición en YAML y la guarda en una colección, que crea si no existe; misma carpeta y nombre = la actualiza)
y `martino_guia` (esta guía). Si la colección que necesitas no existe, ármala con `martino_enviar`: una llamada por endpoint,
con `nombre` claro y tokens como `{{variables}}` del entorno, nunca escritos en la petición.

- Binario: `martino` en el PATH, o `./target/release/martino` dentro del repo (`cargo build --release -p martino`).
- Carpeta de trabajo (`--root`): la que tiene `collections/` y `flows/`. En el repo es `.`.
- En los comandos de abajo, `M` = `martino --root <carpeta>`.

## 1. Mira qué hay antes de escribir

```bash
ls <root>/collections/                         # colecciones (el id es el nombre de la carpeta)
find <root>/collections/<col> -name '*.yml' -o -name '*.bru' | grep -v -e folder -e collection -e environments
ls <root>/collections/<col>/environments/      # entornos: el nombre del archivo sin extensión
head -3 <root>/collections/<col>/tablas/*.csv  # tablas de equivalencia (primera fila = columnas)
ls <root>/flows/ && M flow-yaml <id-parecido>  # un flujo existente en YAML: la mejor plantilla
```

- Lee la petición que vas a usar: sus `{{variables}}` son las que tendrás que pasar en `vars`.
- Si el servicio no existe en la colección, usa una petición inline (`request:`), no inventes archivos.
- Ejemplos que funcionan: `gratis--tutorial-1` (servicio), `-2` (condición), `-3` (tabla), `gratis--viaje` (paralelo + transformar), `gratis--pago-cifrado` (cifrado).

## 2. Escribe el YAML

```yaml
nombre: Consulta de cliente           # visible en la UI
coleccion: gratis                      # id de la carpeta en collections/
inicio: [valida]                       # claves de los primeros pasos (crea el nodo Inicio)
nodos:
  - clave: entrada                     # snake_case, única; es el nombre con que la referencias
    tipo: entrada
    nombre: Entrada
    data: { dni: "12345678", monto: 100 }   # campos y valores de ejemplo
  - clave: valida
    tipo: condicion
    nombre: ¿DNI de 8 dígitos?
    rules: [{ left: entrada.dni, op: isNotEmpty, right: "" }]
    si: [consulta]
    no: [rechazo]
  # …
casos:
  - id: CLI-001
    name: DNI válido
    input: { dni: "12345678", monto: 100 }
    expectStatus: 200
    expect: [{ left: nodo, op: eq, right: ok }]
```

Con `orden: carriles` en la raíz, los pasos quedan en carriles por sistema: una fila para Martino, una por cada sistema que se llama y una para las respuestas (ejemplo: `ejemplos/banco/pago-tarjeta-carriles.yml`). Sin esa línea, se ordenan por etapa.

No pongas `id`, `pos` ni `sid`: el import ordena los pasos en carriles y les asigna códigos (SRV-01…).

### Tipos de paso

| `tipo` | Campos | Salidas | Lo que deja para los siguientes |
|---|---|---|---|
| `entrada` | `data` (objeto de ejemplo) | — | `entrada.campo` |
| `datos` | `data` (admite `{{…}}`) | `siguiente` | `clave.campo` |
| `servicio` | `ref: Carpeta/Archivo.yml` **o** `request: {method, url, headers, bodyMode, body}`; `vars` | `siguiente` (<400), `error` (≥400 o red) | `clave.response.x`, `clave.status`, `clave.headers.X` |
| `extraer` | `vars: [{name, value: ruta}]` | `siguiente` | `clave.name` y la variable `{{name}}` |
| `transformar` | `code` (JS que hace `return {...}`) | `siguiente` | `clave.campo` |
| `condicion` | `rules: [{left, op, right}]`, `mode: all\|any` | `si`, `no` | — |
| `equivalencia` | `lookup: {table, value, column, onMiss: error\|keep\|default, default}` | `siguiente` | la fila: `clave.columna` |
| `cifrado` | `crypto: {op, format, alg, enc, key, input, output}` | `siguiente` | `{{clave}}` (texto o JSON) |
| `respuesta` | `status: "200"` (texto), `body` como objeto YAML, no como texto JSON (`"{{clave}}"` devuelve un objeto entero) | — | termina el flujo: la primera respuesta que llega gana |
| `fin` | — | — | corta el flujo sin responder |
| `nota` | `text` | — | sólo se ve en el lienzo |

Detalles que fallan si no los sabes:
- **Servicio con `ref`:** `vars` llena las `{{variables}}` de la petición: `[{name: lat, value: "{{entrada.lat}}"}]`.
- **Servicio inline:** `request: { method: POST, url: "{{baseUrl}}/pagos", bodyMode: json, body: { json: "{\"monto\": {{entrada.monto}}}" }, headers: [{name: X-Canal, value: web}] }`.
- **Rutas sin llaves:** `extraer.vars[].value` y `condicion.rules[].left` van sin llaves: `cambio.response.rates.PEN`.
- **Valores con llaves:** `right`, `body` y `data` usan `{{…}}` cuando son valores.
- **`transformar`:** recibe `input` (la entrada), `nodes.<clave>` (lo que dejó cada paso; en servicios: `nodes.x.response`, `.status`) y `env`.
- **Paralelo:** dos pasos en el mismo `siguiente`/`si` corren en paralelo. Un paso con varias flechas entrantes espera hasta que ninguna rama en curso pueda llegar a él (une el paralelo sin trabarse con ramas no tomadas).
- **`equivalencia`:** `column` es la columna donde se busca. Si falta, se busca en la primera.
- **Tabla en una plantilla:** `{{tabla.paises[codigo=entrada.codigo].moneda}}`. Sin `col=`, busca en la primera columna.
- **`cifrado` (`format`/`alg`):** `jwe` (RSA-OAEP-256, A256KW, dir) · `jws` (RS256, PS256, HS256) · `aes-gcm` · `aes-cbc` · `rsa-oaep` · `hmac` (SHA-256) · `hash` · `base64`.
- **`cifrado` (`op`):** `encrypt`, `decrypt`, `sign` o `verify`.
- **Llaves y tokens:** siempre como `{{secreto}}` del entorno, nunca escritos en el YAML.

### Operadores (`op`)
- **Comparar:** `eq` `neq` `gt` `gte` `lt` `lte`.
- **Texto y listas:** `contains` `notContains` `startsWith` `endsWith` `in` `notIn`.
- **Existencia y tipo:** `isDefined` `isUndefined` `isNull` `isNotNull` `isEmpty` `isNotEmpty` `isTruthy` `isFalsy` `isNumber` `isString`.
- **`right`:** vacío en los que no comparan.

### Casos QA
- **`input`:** reemplaza la entrada.
- **`expectStatus`:** el status final esperado.
- **`expect[].left`:**
  - `status`;
  - `ms`;
  - `nodo` (la clave de la respuesta que terminó);
  - o una ruta del body (`error.codigo`).
- **Qué cubrir:** al menos un caso por cada rama (`si` y `no`, `siguiente` y `error`) que se pueda provocar con la entrada.

## 3. Insértalo

```bash
M flow-import flujo.yml --id <col>--<nombre-corto>   # o `-` para leer de stdin
```

- Con `--id` reemplaza el flujo si ya existe. Sin `--id`, crea uno nuevo a partir del nombre.
- Si falla, lista lo que no se podría ejecutar: conexión rota, petición o tabla inexistente, clave repetida o servicio sin `ref`/`request`. Corrige el YAML y repite; no edites el `.flow.json` a mano.

## 4. Ejecútalo hasta que pase

```bash
M flow <id> --cases --env <entorno>          # todos los casos; código 0 = todos OK
M flow <id> --env <entorno> --input e.json   # una corrida suelta, muestra cada paso
```

Cada paso imprime `✓/✗ clave (tipo) status ms error`. Ante un `✗` o un caso que no da OK:

| Síntoma | Causa típica |
|---|---|
| `{{x}}` llega literal al servicio | La variable no está en el entorno ni en `vars`: revisa `--env` y las variables de la petición |
| Rama `error` en un servicio | Status ≥400: mira la URL ya resuelta y el body |
| La condición va siempre a `no` | `left` con llaves o con una ruta mal escrita: debe ser una ruta sin llaves |
| `equivalencia` da el default | Buscaste en la columna equivocada: fija `column` |
| `status inválido` | `status` de la respuesta no es un número tras resolver la plantilla |

Itera: corrige el YAML, vuelve a `flow-import` con el mismo `--id` y ejecuta otra vez.

## Explicar un servicio (pedido de un analista funcional)

Cuando pregunten «¿qué hace este servicio?» o «¿cómo funciona por dentro?», el flujo ES el flujograma:
1. Lee el código o las peticiones (`martino_peticion` trae `docs`). Anota cada regla con su `archivo:línea`.
2. Un paso por llamada real, en el orden en que ocurre. En su `desc`: sistema, método y ruta, qué decide y qué códigos de error da.
3. Un paso `nota` «Valores que viajan»: qué valor sale de qué paso y en cuál se reutiliza (`retencionId: Retener → Pagar`).
4. Un caso QA por regla que quieras probar; si el comportamiento sorprende, el caso lo deja demostrado.
5. Ejecuta con `detalle` y responde con: la secuencia de pasos, la tabla de valores, los hallazgos (lo que el código hace distinto de lo que se espera) y el id del flujo para verlo en la UI.

Ejemplos: `banco--pago-tarjeta` (integración cuentas → core de tarjetas, `ejemplos/banco/`) y
`json-server--relaciones-y-borrado` (lógica interna de typicode/json-server, `ejemplos/json-server/`).
Los casos de un mismo flujo corren en paralelo: si comparten estado en el servicio (crear y borrar), usa un solo caso.

## 5. Entrega

- **Cierre:** reporta el id, la ruta `flows/<id>.flow.json`, los casos y su resultado, y el comando para repetirlo.
- **En la UI:** el flujo aparece en Flujos (barra lateral, filtro «Flujos») al refrescar. «Encuadrar» (⊡) lo centra.
- **Lo que no está probado** (un servicio que necesita credenciales que no tienes, una rama sin caso): dilo explícitamente; no lo des por bueno.
