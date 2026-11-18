//! Colecciones en disco con la estructura de Bruno. Los secretos van aparte en
//! `.workbench/secrets.json`; borrar una colección la manda a la papelera.
use martino_core::bru;
use martino_core::yaml_req;
use martino_core::model::{Environment, Pair, Request};
use martino_core::postman::{Converted, Item};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::path::{Component, Path, PathBuf};

pub type Res<T> = Result<T, String>;

/// Carpeta de las tablas de equivalencia dentro de la colección.
const TABLES: &str = "tablas";

fn io<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[derive(Clone)]
pub struct Store {
    pub dir: PathBuf,
    pub state: PathBuf,
}

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TreeItem {
    pub name: String,
    pub path: String,
    pub folder: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub method: String,
    /// Código de la petición (`meta.id`).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub id: String,
    pub seq: f64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<TreeItem>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CollectionInfo {
    pub id: String,
    pub name: String,
    pub items: Vec<TreeItem>,
    pub environments: Vec<String>,
}

/// Nombre de archivo seguro y legible.
pub fn slug(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | ' ' | '.') { c } else { '-' })
        .collect::<String>()
        .trim()
        .trim_matches('.')
        .to_string();
    if t.is_empty() { "sin-nombre".into() } else { t }
}

/// Une `rel` a `base` rechazando rutas absolutas y `..` (los datos llegan del navegador).
pub fn safe_join(base: &Path, rel: &str) -> Res<PathBuf> {
    let p = Path::new(rel);
    if rel.contains('\0') || p.components().any(|c| !matches!(c, Component::Normal(_) | Component::CurDir)) {
        return Err(format!("Ruta no permitida: {rel}"));
    }
    Ok(base.join(p))
}

fn rel(base: &Path, p: &Path) -> String {
    p.strip_prefix(base).unwrap_or(p).to_string_lossy().replace('\\', "/")
}

/// Metadatos de carpeta y colección: viven junto a las peticiones pero no lo son.
/// shortcut: folder/collection y los entornos se siguen leyendo y escribiendo como `.bru`;
/// pasar a `folder.yml` / `opencollection.yml` cuando migremos colecciones enteras a YAML.
const META: [&str; 4] = ["folder.bru", "collection.bru", "folder.yml", "opencollection.yml"];

fn is_request_name(fname: &str) -> bool {
    yaml_req::is_request_file(fname) && !META.contains(&fname)
}

fn unique(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let first = dir.join(format!("{stem}{ext}"));
    if !first.exists() {
        return first;
    }
    (2..).map(|i| dir.join(format!("{stem} {i}{ext}"))).find(|p| !p.exists()).unwrap()
}

impl Store {
    pub fn new(dir: PathBuf, state: PathBuf) -> Store {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::create_dir_all(&state);
        Store { dir, state }
    }

    pub fn col_dir(&self, id: &str) -> Res<PathBuf> {
        let d = safe_join(&self.dir, id)?;
        if id.is_empty() || !d.join("bruno.json").exists() {
            return Err(format!("No existe la colección «{id}»"));
        }
        Ok(d)
    }

    fn req_path(&self, col: &str, path: &str) -> Res<PathBuf> {
        if !yaml_req::is_request_file(path) {
            return Err("La petición debe ser un .yml o .bru".into());
        }
        safe_join(&self.col_dir(col)?, path)
    }

    // ---------- colecciones ----------

    pub fn list(&self) -> Vec<CollectionInfo> {
        let mut out = vec![];
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return out };
        for e in rd.flatten() {
            let d = e.path();
            if !d.join("bruno.json").exists() {
                continue;
            }
            let id = e.file_name().to_string_lossy().to_string();
            let name = std::fs::read_to_string(d.join("bruno.json"))
                .ok()
                .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                .and_then(|v| v["name"].as_str().map(String::from))
                .unwrap_or_else(|| id.clone());
            out.push(CollectionInfo { items: self.tree(&d, &d), environments: self.env_names(&id), id, name });
        }
        out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        out
    }

    fn tree(&self, base: &Path, dir: &Path) -> Vec<TreeItem> {
        let mut items = vec![];
        let Ok(rd) = std::fs::read_dir(dir) else { return items };
        for e in rd.flatten() {
            let p = e.path();
            let fname = e.file_name().to_string_lossy().to_string();
            if fname.starts_with('.') || (dir == base && matches!(fname.as_str(), "environments" | TABLES)) {
                continue;
            }
            if p.is_dir() {
                let meta = std::fs::read_to_string(p.join("folder.bru")).map(|s| bru::parse_request(&s)).unwrap_or_default();
                items.push(TreeItem {
                    name: if meta.name.is_empty() { fname } else { meta.name },
                    path: rel(base, &p),
                    folder: true,
                    method: String::new(),
                    id: String::new(),
                    seq: meta.seq.unwrap_or(f64::MAX),
                    items: self.tree(base, &p),
                });
            } else if is_request_name(&fname) {
                let r = std::fs::read_to_string(&p).ok().and_then(|s| yaml_req::parse_file(&fname, &s).ok()).unwrap_or_default();
                items.push(TreeItem {
                    name: if r.name.is_empty() { yaml_req::strip_ext(&fname).unwrap_or(&fname).into() } else { r.name },
                    path: rel(base, &p),
                    folder: false,
                    method: r.method,
                    id: r.id,
                    seq: r.seq.unwrap_or(f64::MAX),
                    items: vec![],
                });
            }
        }
        items.sort_by(|a, b| b.folder.cmp(&a.folder).then(a.seq.total_cmp(&b.seq)).then(a.name.cmp(&b.name)));
        items
    }

    pub fn create_collection(&self, name: &str) -> Res<String> {
        let d = unique(&self.dir, &slug(name), "");
        std::fs::create_dir_all(d.join("environments")).map_err(io)?;
        let meta = json!({"version": "1", "name": name, "type": "collection", "ignore": ["node_modules", ".git"]});
        std::fs::write(d.join("bruno.json"), serde_json::to_string_pretty(&meta).unwrap()).map_err(io)?;
        std::fs::write(d.join("collection.bru"), bru::serialize_request(&Request { name: String::new(), ..Default::default() })).map_err(io)?;
        Ok(d.file_name().unwrap().to_string_lossy().into())
    }

    /// No borra: mueve la colección (y sus secretos) a `.workbench/papelera/<id> <fecha>/`.
    pub fn delete_collection(&self, id: &str) -> Res<PathBuf> {
        let d = self.col_dir(id)?;
        let trash = self.state.join("papelera");
        std::fs::create_dir_all(&trash).map_err(io)?;
        let stamp = chrono::Local::now().format("%Y-%m-%d %H-%M-%S");
        let dest = unique(&trash, &format!("{id} {stamp}"), "");
        std::fs::rename(&d, &dest).map_err(io)?;
        let mut all = self.secrets();
        if let Some(s) = all.remove(id) {
            std::fs::write(dest.join("secrets.json"), serde_json::to_string_pretty(&s).unwrap()).map_err(io)?;
            self.save_secrets(&all)?;
        }
        Ok(dest)
    }

    pub fn root(&self, col: &str) -> Res<Request> {
        let s = std::fs::read_to_string(self.col_dir(col)?.join("collection.bru")).unwrap_or_default();
        Ok(bru::parse_request(&s))
    }

    pub fn put_root(&self, col: &str, r: &Request) -> Res<()> {
        std::fs::write(self.col_dir(col)?.join("collection.bru"), bru::serialize_request(r)).map_err(io)
    }

    /// .env de la colección (lo más bajo en la precedencia).
    pub fn dotenv(&self, col: &str) -> Vec<(String, String)> {
        let Ok(d) = self.col_dir(col) else { return vec![] };
        std::fs::read_to_string(d.join(".env"))
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.trim().to_string(), v.trim().trim_matches('"').to_string()))
            .collect()
    }

    // ---------- peticiones ----------

    pub fn read_request(&self, col: &str, path: &str) -> Res<Request> {
        let p = self.req_path(col, path)?;
        let s = std::fs::read_to_string(&p).map_err(|_| format!("No existe {path}"))?;
        yaml_req::parse_file(path, &s).map_err(|e| format!("{path}: {e}"))
    }

    pub fn write_request(&self, col: &str, path: &str, r: &Request) -> Res<()> {
        let p = self.req_path(col, path)?;
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(io)?;
        }
        std::fs::write(p, yaml_req::serialize_file(path, r)).map_err(io)
    }

    fn folder_requests(&self, col: &str, folder: &str) -> Res<Vec<(String, Request)>> {
        let d = safe_join(&self.col_dir(col)?, folder)?;
        let mut out: Vec<(String, Request)> = std::fs::read_dir(&d)
            .map_err(io)?
            .flatten()
            .filter(|e| e.path().is_file())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|f| is_request_name(f))
            // Un YAML roto se salta: renumerar `seq` lo sobrescribiría con una petición vacía.
            .filter_map(|f| {
                let path = if folder.is_empty() { f.clone() } else { format!("{folder}/{f}") };
                let r = self.read_request(col, &path).ok()?;
                Some((path, r))
            })
            .collect();
        out.sort_by(|a, b| a.1.seq.unwrap_or(f64::MAX).total_cmp(&b.1.seq.unwrap_or(f64::MAX)).then(a.0.cmp(&b.0)));
        Ok(out)
    }

    /// Crea la petición al final de `folder`. Devuelve su ruta.
    pub fn create_request(&self, col: &str, folder: &str, r: &Request) -> Res<String> {
        let d = safe_join(&self.col_dir(col)?, folder)?;
        std::fs::create_dir_all(&d).map_err(io)?;
        let mut r = r.clone();
        let last = self.folder_requests(col, folder)?.iter().filter_map(|x| x.1.seq).fold(0.0, f64::max);
        r.seq = Some(last + 1.0);
        if r.name.is_empty() {
            r.name = "Nueva petición".into();
        }
        r.id = self.next_id(col, folder, &r.id)?;
        let p = unique(&d, &slug(&r.name), yaml_req::EXT);
        let path = rel(&self.col_dir(col)?, &p);
        self.write_request(col, &path, &r)?;
        Ok(path)
    }

    /// ID doble para una petición nueva en `folder`: `wanted` si está libre; si no, el siguiente del prefijo.
    /// shortcut: relee el árbol entero en cada petición nueva (importar N peticiones es O(N²) lecturas);
    /// pasar a un índice de IDs en memoria si una importación grande se nota lenta.
    pub fn next_id(&self, col: &str, folder: &str, wanted: &str) -> Res<String> {
        fn ids(items: &[TreeItem], out: &mut Vec<String>) {
            for i in items {
                out.extend((!i.id.is_empty()).then(|| i.id.clone()));
                ids(&i.items, out);
            }
        }
        let d = self.col_dir(col)?;
        let mut used = vec![];
        ids(&self.tree(&d, &d), &mut used);
        Ok(martino_core::model::resolve_id(wanted, &martino_core::model::id_prefix(folder), &used))
    }

    pub fn delete_request(&self, col: &str, path: &str) -> Res<()> {
        std::fs::remove_file(self.req_path(col, path)?).map_err(io)
    }

    pub fn create_folder(&self, col: &str, parent: &str, name: &str) -> Res<String> {
        let base = safe_join(&self.col_dir(col)?, parent)?;
        let d = unique(&base, &slug(name), "");
        std::fs::create_dir_all(&d).map_err(io)?;
        let r = Request { name: name.into(), seq: Some(1.0), ..Default::default() };
        std::fs::write(d.join("folder.bru"), bru::serialize_request(&r)).map_err(io)?;
        Ok(rel(&self.col_dir(col)?, &d))
    }

    /// Mueve `from` a la carpeta `to_folder` antes de `before` (o al final) y renumera `seq`.
    pub fn move_request(&self, col: &str, from: &str, to_folder: &str, before: Option<&str>) -> Res<String> {
        let src = self.req_path(col, from)?;
        let fname = src.file_name().ok_or("Ruta inválida")?.to_string_lossy().to_string();
        let same_folder = Path::new(from).parent().map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_default() == to_folder;
        let new_path = if same_folder {
            from.to_string()
        } else {
            let d = safe_join(&self.col_dir(col)?, to_folder)?;
            let stem = yaml_req::strip_ext(&fname).unwrap_or(&fname);
            let dest = unique(&d, stem, &fname[stem.len()..]);
            std::fs::rename(&src, &dest).map_err(io)?;
            rel(&self.col_dir(col)?, &dest)
        };
        let mut list: Vec<(String, Request)> = self.folder_requests(col, to_folder)?.into_iter().filter(|x| x.0 != new_path).collect();
        let moved = self.read_request(col, &new_path)?;
        let idx = before.and_then(|b| list.iter().position(|x| x.0 == b)).unwrap_or(list.len());
        list.insert(idx, (new_path.clone(), moved));
        for (i, (p, mut r)) in list.into_iter().enumerate() {
            let seq = Some(i as f64 + 1.0);
            if r.seq != seq {
                r.seq = seq;
                self.write_request(col, &p, &r)?;
            }
        }
        Ok(new_path)
    }

    // ---------- tablas de equivalencia (`tablas/*.csv`, entran a git) ----------

    fn table_path(&self, col: &str, name: &str) -> Res<PathBuf> {
        if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || matches!(c, '-' | '_')) {
            return Err(format!("Nombre de tabla no permitido: «{name}» (usa letras, números, - o _)"));
        }
        Ok(self.col_dir(col)?.join(TABLES).join(format!("{name}.csv")))
    }

    pub fn tables(&self, col: &str) -> Vec<Value> {
        let Ok(d) = self.col_dir(col) else { return vec![] };
        let mut v: Vec<Value> = std::fs::read_dir(d.join(TABLES))
            .map(|rd| rd.flatten().filter_map(|e| {
                let name = e.file_name().to_string_lossy().strip_suffix(".csv")?.to_string();
                Some(json!({"name": name, "csv": std::fs::read_to_string(e.path()).ok()?}))
            }).collect())
            .unwrap_or_default();
        v.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        v
    }

    pub fn read_table(&self, col: &str, name: &str) -> Res<String> {
        std::fs::read_to_string(self.table_path(col, name)?).map_err(|_| format!("No existe la tabla «{name}»"))
    }

    pub fn put_table(&self, col: &str, name: &str, csv: &str) -> Res<()> {
        let p = self.table_path(col, name)?;
        std::fs::create_dir_all(p.parent().unwrap()).map_err(io)?;
        std::fs::write(p, csv).map_err(io)
    }

    pub fn delete_table(&self, col: &str, name: &str) -> Res<()> {
        std::fs::remove_file(self.table_path(col, name)?).map_err(io)
    }

    // ---------- entornos y secretos ----------

    pub fn env_names(&self, col: &str) -> Vec<String> {
        let Ok(d) = self.col_dir(col) else { return vec![] };
        let mut v: Vec<String> = std::fs::read_dir(d.join("environments"))
            .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().to_string()).filter_map(|f| f.strip_suffix(".bru").map(String::from)).collect())
            .unwrap_or_default();
        v.sort();
        v
    }

    fn env_path(&self, col: &str, env: &str) -> Res<PathBuf> {
        if env.is_empty() || env.contains(['/', '\\']) {
            return Err(format!("Nombre de entorno inválido: «{env}»"));
        }
        safe_join(&self.col_dir(col)?.join("environments"), &format!("{env}.bru"))
    }

    pub fn secrets(&self) -> Map<String, Value> {
        std::fs::read_to_string(self.state.join("secrets.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    fn save_secrets(&self, all: &Map<String, Value>) -> Res<()> {
        std::fs::write(self.state.join("secrets.json"), serde_json::to_string_pretty(all).unwrap()).map_err(io)
    }

    pub fn environment(&self, col: &str, env: &str) -> Res<Environment> {
        let src = std::fs::read_to_string(self.env_path(col, env)?).map_err(|_| format!("No existe el entorno «{env}»"))?;
        let mut e = bru::parse_environment(env, &src);
        let all = self.secrets();
        for s in e.secrets.iter_mut() {
            if let Some(v) = all.get(col).and_then(|c| c.get(env)).and_then(|m| m.get(&s.name)).and_then(Value::as_str) {
                s.value = v.into();
            }
        }
        Ok(e)
    }

    /// Guarda el entorno; si `e.name` cambia, lo renombra (con sus secretos).
    pub fn put_environment(&self, col: &str, env: &str, e: &Environment) -> Res<()> {
        let name = if e.name.is_empty() { env } else { e.name.as_str() };
        let path = self.env_path(col, name)?;
        std::fs::create_dir_all(path.parent().unwrap()).map_err(io)?;
        std::fs::write(&path, bru::serialize_environment(e)).map_err(io)?;
        let mut all = self.secrets();
        let c = all.entry(col.to_string()).or_insert(json!({}));
        if name != env {
            if let Some(m) = c.as_object_mut() {
                m.remove(env);
            }
            if let Ok(old) = self.env_path(col, env) {
                let _ = std::fs::remove_file(old);
            }
        }
        c[name] = Value::Object(e.secrets.iter().map(|p| (p.name.clone(), json!(p.value))).collect());
        self.save_secrets(&all)
    }

    pub fn delete_environment(&self, col: &str, env: &str) -> Res<()> {
        std::fs::remove_file(self.env_path(col, env)?).map_err(io)?;
        let mut all = self.secrets();
        if let Some(c) = all.get_mut(col).and_then(Value::as_object_mut) {
            c.remove(env);
        }
        self.save_secrets(&all)
    }

    // ---------- importación ----------

    fn write_items(&self, col: &str, folder: &str, items: &[Item]) -> Res<()> {
        for it in items {
            if it.folder {
                let path = self.create_folder(col, folder, &it.name)?;
                let mut root = it.request.clone();
                root.name = it.name.clone();
                std::fs::write(safe_join(&self.col_dir(col)?, &path)?.join("folder.bru"), bru::serialize_request(&root)).map_err(io)?;
                self.write_items(col, &path, &it.items)?;
            } else {
                self.create_request(col, folder, &it.request)?;
            }
        }
        Ok(())
    }

    /// Escribe una colección convertida. Con `into` la mezcla en una existente.
    pub fn import(&self, c: &Converted, into: Option<&str>) -> Res<String> {
        let col = match into {
            Some(id) => {
                self.col_dir(id)?;
                id.to_string()
            }
            None => self.create_collection(&c.name)?,
        };
        if into.is_none() {
            let mut root = c.root.clone();
            root.name = String::new();
            self.put_root(&col, &root)?;
        }
        self.write_items(&col, "", &c.items)?;
        for e in &c.environments {
            let name = slug(if e.name.is_empty() { "importado" } else { &e.name });
            self.put_environment(&col, &name, &Environment { name: name.clone(), ..e.clone() })?;
        }
        Ok(col)
    }

    // ---------- workspaces ----------

    pub fn workspaces(&self) -> Value {
        std::fs::read_to_string(self.state.join("workspaces.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(json!({}))
    }

    pub fn put_workspaces(&self, v: &Value) -> Res<()> {
        std::fs::write(self.state.join("workspaces.json"), serde_json::to_string_pretty(v).unwrap()).map_err(io)
    }
}

/// Variables de un entorno como pares (incluye secretos).
pub fn env_pairs(e: &Environment) -> Vec<(String, String)> {
    e.vars.iter().filter(|p| p.enabled).map(|p| (p.name.clone(), p.value.clone())).collect()
}

pub fn secret_pairs(e: &Environment) -> Vec<(String, String)> {
    e.secrets.iter().filter(|p| p.enabled).map(|p| (p.name.clone(), p.value.clone())).collect()
}

pub fn enabled(pairs: &[Pair]) -> Vec<(String, String)> {
    pairs.iter().filter(|p| p.enabled).map(|p| (p.name.clone(), p.value.clone())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let d = tempfile::tempdir().unwrap();
        let s = Store::new(d.path().join("c"), d.path().join(".wb"));
        (d, s)
    }

    #[test]
    fn crud_move_and_trash() {
        let (_d, s) = store();
        let col = s.create_collection("Demo").unwrap();
        let f = s.create_folder(&col, "", "Usuarios").unwrap();
        let a = s.create_request(&col, &f, &Request { name: "A".into(), ..Request::new("get", "/a") }).unwrap();
        let b = s.create_request(&col, &f, &Request { name: "B".into(), ..Request::new("get", "/b") }).unwrap();
        let c = s.create_request(&col, "", &Request { name: "C".into(), ..Request::new("post", "/c") }).unwrap();
        assert_eq!((a.as_str(), s.read_request(&col, &b).unwrap().seq), ("Usuarios/A.yml", Some(2.0)));
        assert_eq!(s.read_request(&col, &b).unwrap().id, "USU-002");
        let agent = s.create_request(&col, &f, &Request { name: "D".into(), id: "PAG-014".into(), ..Request::new("get", "/d") }).unwrap();
        assert_eq!(s.read_request(&col, &agent).unwrap().id, "PAG-014", "respeta el ID del agente");
        assert_eq!(s.next_id(&col, &f, "PAG-014").unwrap(), "USU-003", "ocupado → siguiente libre");
        s.delete_request(&col, &agent).unwrap();
        s.put_table(&col, "bancos", "codigo,nombre\n002,BCP\n").unwrap();
        assert_eq!(s.tables(&col)[0]["name"], "bancos");
        assert!(s.put_table(&col, "../x", "").is_err());
        assert_eq!(s.list()[0].items.iter().filter(|i| i.name == "tablas").count(), 0, "las tablas no son carpetas de peticiones");
        // C entra a la carpeta antes de B
        let moved = s.move_request(&col, &c, &f, Some(&b)).unwrap();
        assert_eq!(moved, "Usuarios/C.yml");
        let order: Vec<_> = s.folder_requests(&col, &f).unwrap().into_iter().map(|x| (x.0, x.1.seq)).collect();
        assert_eq!(order, vec![(a.clone(), Some(1.0)), (moved.clone(), Some(2.0)), (b.clone(), Some(3.0))]);
        // reordenar dentro de la carpeta: A al final
        s.move_request(&col, &a, &f, None).unwrap();
        assert_eq!(s.read_request(&col, &a).unwrap().seq, Some(3.0));
        let tree = &s.list()[0];
        assert_eq!(tree.items[0].items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["C", "B", "A"]);
        assert!(s.read_request(&col, "../x.bru").is_err());
        assert!(s.read_request(&col, "bruno.json").is_err(), "sólo archivos de petición");
        // un .bru existente se lee, se mueve y se guarda como .bru
        let dir = s.col_dir(&col).unwrap();
        std::fs::write(dir.join("Vieja.bru"), bru::serialize_request(&Request { name: "Vieja".into(), ..Request::new("get", "/v") })).unwrap();
        let old = s.move_request(&col, "Vieja.bru", &f, None).unwrap();
        assert_eq!(old, "Usuarios/Vieja.bru");
        assert!(std::fs::read_to_string(dir.join(&old)).unwrap().contains("meta {"), "sigue en formato .bru");
        assert_eq!(s.list()[0].items[0].items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["C", "B", "A", "Vieja"]);
        s.delete_request(&col, &old).unwrap();
        assert!(safe_join(Path::new("/a"), "/etc/passwd").is_err());

        let env = Environment { name: "dev".into(), vars: vec![Pair::new("url", "u")], secrets: vec![Pair::new("token", "T")], ..Default::default() };
        s.put_environment(&col, "dev", &env).unwrap();
        let disk = std::fs::read_to_string(s.col_dir(&col).unwrap().join("environments/dev.bru")).unwrap();
        assert!(!disk.contains('T') || !disk.contains(": T"));
        assert_eq!(s.environment(&col, "dev").unwrap().secrets[0].value, "T");
        s.put_environment(&col, "dev", &Environment { name: "qa".into(), ..env }).unwrap();
        assert_eq!(s.env_names(&col), ["qa"]);
        assert_eq!(s.environment(&col, "qa").unwrap().secrets[0].value, "T");

        let dest = s.delete_collection(&col).unwrap();
        assert!(dest.join("bruno.json").exists() && dest.join("secrets.json").exists());
        assert!(s.list().is_empty());
        assert!(s.secrets().get(&col).is_none());
    }
}
