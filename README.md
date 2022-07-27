# Buscame Servidor API

REST API with Rust and MongoDB
 
- Crear usuario (api-buscame.vercel.app/create-user): verifica con correo electrónico la autenticidad del usuario y crea un nuevo rol
- Eliminar Usuario (api-buscame.vercel.app/delete-user): Elimina el usuario y/o sub-usuario
- Agregar Nuevo Subusuario (api-buscame.vercel.app/create-new-subuser): agregue un subusuario en un usuario maestro
- Solicitar PDF (api-buscame.vercel.app/{user}/pdf): Recibe la solicitud del BOT a un RUT o Nombre ya solicitado o crear un nuevo doc con la informacion aderida
- Solicitud de Proceso (api-buscame.vercel.app/{user}/{pdf-number} or {all}): Procesa y manipule los datos en un PDF y cree un cron para enviar notificaciones al correo del cliente y del usuario 
- Solicitar Batch PDFs (api-buscame.vercel.app/{user}/{batch-pdf.exe}): Procesa y cataloga RUT o Nombres que se soliciten en lotes (hasta 1k), notifica si se encuentra con algún error, y en caso contrario envia y gestiona con el BOT la creación de nuevos docs para cada uno de los Rut's solicitados.
